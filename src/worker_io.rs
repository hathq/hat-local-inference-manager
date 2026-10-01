use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const MAX_COMMAND_OUTPUT_BYTES: u64 = 1_048_576;

pub(super) fn arguments() -> Result<BTreeMap<String, String>, String> {
    let mut values = BTreeMap::new();
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let key = flag
            .strip_prefix("--")
            .ok_or("worker arguments must be named")?;
        let value = args.next().ok_or("worker argument value is missing")?;
        if values.insert(key.to_owned(), value).is_some() {
            return Err("worker argument is duplicated".into());
        }
    }
    Ok(values)
}

pub(super) fn required<'a>(
    values: &'a BTreeMap<String, String>,
    key: &str,
) -> Result<&'a str, String> {
    values
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| format!("--{key} is required"))
}

pub(super) fn run_hatter(
    values: &BTreeMap<String, String>,
    command: &str,
    common: &[String],
    extra: &[&str],
) -> Result<Vec<u8>, String> {
    let timeout_seconds = values
        .get("hatter-timeout-seconds")
        .map_or(Ok(15), |value| value.parse::<u64>())
        .map_err(message)?;
    if !(1..=120).contains(&timeout_seconds) {
        return Err("--hatter-timeout-seconds must be between 1 and 120".into());
    }
    let mut child = Command::new(required(values, "hatter")?)
        .env("HATTER_HOME", required(values, "hatter-home")?)
        .arg("hat")
        .arg(command)
        .args(common)
        .args(extra)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(message)?;
    let Some(stdout_pipe) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("hatter stdout is unavailable".into());
    };
    let Some(stderr_pipe) = child.stderr.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("hatter stderr is unavailable".into());
    };
    let stdout = pipe_reader(stdout_pipe);
    let stderr = pipe_reader(stderr_pipe);
    let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
    let status = loop {
        match child.try_wait() {
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout.join();
                let _ = stderr.join();
                return Err(error.to_string());
            }
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout.join();
                let _ = stderr.join();
                return Err(format!("hatter {command} exceeded its bounded timeout"));
            }
        }
    };
    let stdout = join_reader(stdout)?;
    let stderr = join_reader(stderr)?;
    if !status.success() {
        return Err(format!(
            "hatter {command} failed: {}",
            String::from_utf8_lossy(&stderr)
        ));
    }
    Ok(stdout)
}

fn pipe_reader(pipe: impl Read + Send + 'static) -> thread::JoinHandle<Result<Vec<u8>, String>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        pipe.take(MAX_COMMAND_OUTPUT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(message)?;
        if bytes.len() as u64 > MAX_COMMAND_OUTPUT_BYTES {
            return Err("hatter command output exceeds 1 MiB".into());
        }
        Ok(bytes)
    })
}

fn join_reader(reader: thread::JoinHandle<Result<Vec<u8>, String>>) -> Result<Vec<u8>, String> {
    reader
        .join()
        .map_err(|_| "hatter output reader panicked".to_owned())?
}

pub(super) fn canonical_directory(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    let metadata = fs::symlink_metadata(path).map_err(message)?;
    let canonical = fs::canonicalize(path).map_err(message)?;
    if !path.is_absolute()
        || metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || canonical != path
    {
        return Err("worker directory must be one exact absolute directory".into());
    }
    Ok(canonical)
}

pub(super) fn write_document(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > 1_048_576 {
        return Err("worker document exceeds 1 MiB".into());
    }
    if path.exists() {
        return if fs::read(path).map_err(message)? == bytes {
            Ok(())
        } else {
            Err("worker document already differs".into())
        };
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(message)?;
    file.write_all(bytes).map_err(message)?;
    file.sync_all().map_err(message)
}

pub(super) fn path(value: &Path) -> Result<&str, String> {
    value
        .to_str()
        .ok_or_else(|| "worker path is not UTF-8".into())
}

pub(super) fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(all(test, unix))]
mod tests {
    use super::run_hatter;
    use std::collections::BTreeMap;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(1);

    #[test]
    fn bounded_command_timeout_terminates_and_reaps_the_control_process() {
        let directory = std::env::temp_dir().join(format!(
            "hat-local-inference-command-timeout-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).expect("temporary directory");
        let executable = directory.join("slow-hatter");
        std::fs::write(&executable, b"#!/bin/sh\nexec /usr/bin/sleep 5\n")
            .expect("test executable");
        let mut permissions = std::fs::metadata(&executable)
            .expect("test executable metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&executable, permissions).expect("test executable mode");
        let values = BTreeMap::from([
            ("hatter".into(), executable.to_string_lossy().into_owned()),
            (
                "hatter-home".into(),
                directory.to_string_lossy().into_owned(),
            ),
            ("hatter-timeout-seconds".into(), "1".into()),
        ]);
        let started = Instant::now();
        let error = run_hatter(&values, "status", &[], &[]).expect_err("timeout");
        assert!(error.contains("bounded timeout"));
        assert!(started.elapsed() < Duration::from_secs(3));
        std::fs::remove_dir_all(directory).expect("remove temporary directory");
    }
}
