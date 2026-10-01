use std::collections::BTreeMap;
use std::fs::{self, OpenOptions, TryLockError};
use std::path::Path;
use std::thread::{self, JoinHandle};
use std::time::Duration;

const REQUIRED_ARGUMENTS: [&str; 12] = [
    "artifact-store",
    "catalog-state",
    "context-partition-id",
    "hatter",
    "hatter-home",
    "input-store",
    "model-root",
    "output-store",
    "placement-digest",
    "state-dir",
    "worker-id",
    "worker-identity-ref",
];
const OPTIONAL_ARGUMENTS: [&str; 5] = [
    "hatter-timeout-seconds",
    "idle-timeout-seconds",
    "lease-ttl-seconds",
    "max-claims",
    "max-concurrency",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SupervisorConfig {
    pub(super) max_concurrency: usize,
    pub(super) max_claims: u64,
    pub(super) idle_timeout: Option<Duration>,
    pub(super) lease_ttl_seconds: u64,
    pub(super) heartbeat_interval: Duration,
    pub(super) poll_interval: Duration,
}

impl SupervisorConfig {
    pub(super) fn parse(values: &BTreeMap<String, String>) -> Result<Self, String> {
        for required in REQUIRED_ARGUMENTS {
            if !values.contains_key(required) {
                return Err(format!("--{required} is required"));
            }
        }
        if let Some(unknown) = values.keys().find(|key| {
            !REQUIRED_ARGUMENTS.contains(&key.as_str())
                && !OPTIONAL_ARGUMENTS.contains(&key.as_str())
        }) {
            return Err(format!("--{unknown} is not a worker argument"));
        }
        let default_concurrency = thread::available_parallelism()
            .map_or(1, std::num::NonZero::get)
            .clamp(1, 4);
        let max_concurrency = parse(values, "max-concurrency", default_concurrency)?;
        if !(1..=16).contains(&max_concurrency) {
            return Err("--max-concurrency must be between 1 and 16".into());
        }
        let max_claims = parse(values, "max-claims", 0_u64)?;
        let idle_timeout_seconds = parse(values, "idle-timeout-seconds", 0_u64)?;
        if idle_timeout_seconds > 86_400 {
            return Err("--idle-timeout-seconds exceeds one day".into());
        }
        let lease_ttl_seconds = parse(values, "lease-ttl-seconds", 15_u64)?;
        if !(5..=300).contains(&lease_ttl_seconds) {
            return Err("--lease-ttl-seconds must be between 5 and 300".into());
        }
        Ok(Self {
            max_concurrency,
            max_claims,
            idle_timeout: (idle_timeout_seconds > 0)
                .then(|| Duration::from_secs(idle_timeout_seconds)),
            lease_ttl_seconds,
            heartbeat_interval: Duration::from_secs((lease_ttl_seconds / 3).max(1)),
            poll_interval: Duration::from_millis(50),
        })
    }
}

fn parse<T>(values: &BTreeMap<String, String>, key: &str, default: T) -> Result<T, String>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    values.get(key).map_or(Ok(default), |value| {
        value.parse::<T>().map_err(|error| error.to_string())
    })
}

pub(super) struct ProcessLock {
    file: fs::File,
}

impl ProcessLock {
    pub(super) fn acquire(state_directory: &Path) -> Result<Self, String> {
        let path = state_directory.join("worker-supervisor.lock");
        let file = match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err("worker supervisor lock is not a regular file".into());
                }
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&path)
                    .map_err(|error| error.to_string())?
            }
            Err(error) => return Err(error.to_string()),
        };
        match file.try_lock() {
            Ok(()) => Ok(Self { file }),
            Err(TryLockError::WouldBlock) => {
                Err("another worker supervisor already owns this state directory".into())
            }
            Err(TryLockError::Error(error)) => Err(error.to_string()),
        }
    }
}

impl Drop for ProcessLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub(super) struct Finished<M, T> {
    pub(super) metadata: M,
    pub(super) result: Result<T, String>,
}

struct Active<M, T> {
    metadata: M,
    handle: JoinHandle<Result<T, String>>,
}

pub(super) struct WorkerPool<M, T> {
    limit: usize,
    active: Vec<Active<M, T>>,
}

impl<M, T> WorkerPool<M, T>
where
    M: Send + 'static,
    T: Send + 'static,
{
    pub(super) fn new(limit: usize) -> Self {
        Self {
            limit,
            active: Vec::with_capacity(limit),
        }
    }

    pub(super) fn active_len(&self) -> usize {
        self.active.len()
    }

    pub(super) fn has_capacity(&self) -> bool {
        self.active.len() < self.limit
    }

    pub(super) fn spawn(
        &mut self,
        metadata: M,
        work: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Result<(), String> {
        if !self.has_capacity() {
            return Err("worker concurrency limit reached".into());
        }
        self.active.push(Active {
            metadata,
            handle: thread::spawn(work),
        });
        Ok(())
    }

    pub(super) fn reap_finished(&mut self) -> Vec<Finished<M, T>> {
        let mut finished = Vec::new();
        let mut index = 0;
        while index < self.active.len() {
            if self.active[index].handle.is_finished() {
                let active = self.active.swap_remove(index);
                finished.push(Finished {
                    metadata: active.metadata,
                    result: active
                        .handle
                        .join()
                        .map_err(|_| "worker task panicked".to_owned())
                        .and_then(std::convert::identity),
                });
            } else {
                index += 1;
            }
        }
        finished
    }

    pub(super) fn drain(&mut self) -> Vec<Finished<M, T>> {
        let mut finished = Vec::with_capacity(self.active.len());
        while let Some(active) = self.active.pop() {
            finished.push(Finished {
                metadata: active.metadata,
                result: active
                    .handle
                    .join()
                    .map_err(|_| "worker task panicked".to_owned())
                    .and_then(std::convert::identity),
            });
        }
        finished
    }
}

#[cfg(test)]
mod tests {
    use super::{ProcessLock, SupervisorConfig, WorkerPool};
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use std::time::Duration;

    fn arguments() -> BTreeMap<String, String> {
        super::REQUIRED_ARGUMENTS
            .into_iter()
            .map(|key| (key.to_owned(), "value".to_owned()))
            .collect()
    }

    #[test]
    fn configuration_is_bounded_and_rejects_unknown_arguments() {
        let mut values = arguments();
        values.insert("max-concurrency".into(), "0".into());
        assert!(SupervisorConfig::parse(&values).is_err());
        values.insert("max-concurrency".into(), "2".into());
        values.insert("lease-ttl-seconds".into(), "12".into());
        values.insert("idle-timeout-seconds".into(), "3".into());
        let config = SupervisorConfig::parse(&values).expect("bounded config");
        assert_eq!(config.max_concurrency, 2);
        assert_eq!(config.heartbeat_interval, Duration::from_secs(4));
        assert_eq!(config.idle_timeout, Some(Duration::from_secs(3)));
        values.insert("legacy-option".into(), "true".into());
        assert!(SupervisorConfig::parse(&values).is_err());
    }

    #[test]
    fn process_lock_prevents_duplicate_supervisors_and_releases_on_drop() {
        let directory = std::env::temp_dir().join(format!(
            "hat-local-inference-worker-lock-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("temporary directory");
        let first = ProcessLock::acquire(&directory).expect("first owner");
        assert!(ProcessLock::acquire(&directory).is_err());
        drop(first);
        ProcessLock::acquire(&directory).expect("replacement owner");
        std::fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn worker_pool_never_exceeds_its_limit_and_joins_panics() {
        let mut pool = WorkerPool::new(2);
        let barrier = Arc::new(Barrier::new(3));
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        for metadata in 0..2 {
            let barrier = Arc::clone(&barrier);
            let active = Arc::clone(&active);
            let maximum = Arc::clone(&maximum);
            pool.spawn(metadata, move || {
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                maximum.fetch_max(current, Ordering::SeqCst);
                barrier.wait();
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(metadata)
            })
            .expect("bounded task");
        }
        assert!(pool.spawn(3, || Ok(3)).is_err());
        barrier.wait();
        let finished = pool.drain();
        assert_eq!(finished.len(), 2);
        assert_eq!(maximum.load(Ordering::SeqCst), 2);

        pool.spawn(4, || -> Result<usize, String> { panic!("test panic") })
            .expect("panic task");
        let panicked = pool.drain();
        assert_eq!(panicked.len(), 1);
        assert_eq!(panicked[0].metadata, 4);
        assert!(matches!(
            &panicked[0].result,
            Err(error) if error == "worker task panicked"
        ));
    }
}
