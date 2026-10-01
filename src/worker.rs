use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Instant;

use hat_local_inference_manager::{
    Action, ActionRequest, OUTPUT_SCHEMA, REPOSITORY_ID, Storage, execute, operation,
};
use hat_specifications::{
    ACTION_RESULT_SCHEMA, ACTION_STATUS_SCHEMA, ActionReference, HatActionResult, HatActionStatus,
    HatInvocation, HatInvocationOutcome, HatInvocationPhase,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::supervisor::{Finished, ProcessLock, SupervisorConfig, WorkerPool};
use crate::worker_io::{
    arguments, canonical_directory, message, path, required, run_hatter, write_document,
};

#[derive(Clone, Deserialize)]
struct Lease {
    record: LeaseRecord,
}

#[derive(Clone, Deserialize)]
struct LeaseRecord {
    status: HatActionStatus,
    invocation: HatInvocation,
}

#[derive(Deserialize)]
struct RegistrationLease {
    worker_id: String,
    revision: u64,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkerSummary {
    schema: &'static str,
    max_concurrency: usize,
    claimed: u64,
    completed: u64,
    unresolved: u64,
}

struct SupervisorExit {
    summary: WorkerSummary,
    error: Option<String>,
}

pub(super) fn run() -> Result<(), String> {
    let args = Arc::new(arguments()?);
    let config = SupervisorConfig::parse(&args)?;
    let control = canonical_directory(required(&args, "state-dir")?)?;
    let _process_lock = ProcessLock::acquire(&control)?;
    cleanup_transient_documents(&control)?;
    let common = Arc::new(vec![
        "--context-partition-id".into(),
        required(&args, "context-partition-id")?.into(),
        "--repository-id".into(),
        REPOSITORY_ID.into(),
    ]);
    let mut registration_revision =
        register(&args, &common, &control, config.lease_ttl_seconds, 0)?;
    let exit = supervise(&args, &common, &control, config, &mut registration_revision);
    let unregister_result = unregister(&args, &common, registration_revision);
    println!("{}", serde_json::to_string(&exit.summary).map_err(message)?);
    match (exit.error, unregister_result) {
        (None, Ok(())) => Ok(()),
        (Some(error), Ok(())) | (None, Err(error)) => Err(error),
        (Some(error), Err(cleanup)) => Err(format!("{error}; worker unregister failed: {cleanup}")),
    }
}

fn supervise(
    args: &Arc<BTreeMap<String, String>>,
    common: &Arc<Vec<String>>,
    control: &Path,
    config: SupervisorConfig,
    registration_revision: &mut u64,
) -> SupervisorExit {
    let mut summary = WorkerSummary {
        schema: "hathq://hat-local-inference-manager/worker-supervisor-summary/v1",
        max_concurrency: config.max_concurrency,
        ..WorkerSummary::default()
    };
    let mut pool = WorkerPool::<Lease, String>::new(config.max_concurrency);
    let storage_gate = Arc::new(RwLock::new(()));
    let mut last_activity = Instant::now();
    let mut next_heartbeat = Instant::now() + config.heartbeat_interval;
    let mut fatal = None;
    loop {
        for finished in pool.reap_finished() {
            last_activity = Instant::now();
            if let Err(error) = finish_task(args, control, finished, &mut summary) {
                fatal.get_or_insert(error);
            }
        }
        if Instant::now() >= next_heartbeat {
            match register(
                args,
                common,
                control,
                config.lease_ttl_seconds,
                *registration_revision,
            ) {
                Ok(revision) => *registration_revision = revision,
                Err(error) => {
                    fatal.get_or_insert(format!("worker heartbeat failed: {error}"));
                }
            }
            next_heartbeat = Instant::now() + config.heartbeat_interval;
        }
        let claim_limit_reached = config.max_claims > 0 && summary.claimed >= config.max_claims;
        if fatal.is_none() && !claim_limit_reached && pool.has_capacity() {
            match claim(args, common) {
                Ok(Some(lease)) => {
                    summary.claimed += 1;
                    last_activity = Instant::now();
                    let task_args = Arc::clone(args);
                    let task_common = Arc::clone(common);
                    let task_control = control.to_path_buf();
                    let task_lease = lease.clone();
                    let task_gate = Arc::clone(&storage_gate);
                    if let Err(error) = pool.spawn(lease, move || {
                        process(
                            &task_args,
                            &task_common,
                            &task_control,
                            &task_lease,
                            &task_gate,
                        )
                    }) {
                        fatal.get_or_insert(error);
                    }
                    continue;
                }
                Ok(None) => {}
                Err(error) => {
                    fatal.get_or_insert(format!("worker claim failed: {error}"));
                }
            }
        }
        let idle_expired = config
            .idle_timeout
            .is_some_and(|timeout| last_activity.elapsed() >= timeout);
        let should_stop =
            fatal.is_some() || claim_limit_reached || (idle_expired && pool.active_len() == 0);
        if should_stop && pool.active_len() == 0 {
            break;
        }
        thread::sleep(config.poll_interval);
    }
    for finished in pool.drain() {
        if let Err(error) = finish_task(args, control, finished, &mut summary) {
            fatal.get_or_insert(error);
        }
    }
    SupervisorExit {
        summary,
        error: fatal,
    }
}

fn register(
    args: &BTreeMap<String, String>,
    common: &[String],
    control: &Path,
    ttl_seconds: u64,
    expected_revision: u64,
) -> Result<u64, String> {
    let registration = serde_json::json!({
        "worker_id": required(args, "worker-id")?,
        "worker_identity_ref": required(args, "worker-identity-ref")?,
        "placement_selection_digest_sha256": required(args, "placement-digest")?,
        "expected_revision": expected_revision,
        "ttl_seconds": ttl_seconds
    });
    let bytes = serde_json::to_vec(&registration).map_err(message)?;
    let registration_path = document_path(control, "registration", &bytes);
    write_document(&registration_path, &bytes)?;
    let response = remove_transient_output(
        &registration_path,
        run_hatter(
            args,
            "worker-register",
            common,
            &["--registration-json", path(&registration_path)?],
        ),
    )?;
    let lease: RegistrationLease = serde_json::from_slice(&response).map_err(message)?;
    if lease.worker_id != required(args, "worker-id")? || lease.revision == 0 {
        return Err("worker registration response differs from the requested owner".into());
    }
    Ok(lease.revision)
}

fn unregister(
    args: &BTreeMap<String, String>,
    common: &[String],
    revision: u64,
) -> Result<(), String> {
    let revision = revision.to_string();
    run_hatter(
        args,
        "worker-unregister",
        common,
        &[
            "--worker-id",
            required(args, "worker-id")?,
            "--expected-revision",
            &revision,
        ],
    )?;
    Ok(())
}

fn claim(args: &BTreeMap<String, String>, common: &[String]) -> Result<Option<Lease>, String> {
    let claim = run_hatter(
        args,
        "claim",
        common,
        &["--worker-id", required(args, "worker-id")?],
    )?;
    serde_json::from_slice(&claim).map_err(message)
}

fn finish_task(
    args: &BTreeMap<String, String>,
    control: &Path,
    finished: Finished<Lease, String>,
    summary: &mut WorkerSummary,
) -> Result<(), String> {
    match finished.result {
        Ok(_) => {
            summary.completed += 1;
            Ok(())
        }
        Err(error) => {
            summary.unresolved += 1;
            mark_unresolved(args, control, &finished.metadata).map_err(|transition| {
                format!(
                    "invocation {} failed ({error}) and unresolved transition failed ({transition})",
                    finished.metadata.record.invocation.invocation_id
                )
            })
        }
    }
}

fn mark_unresolved(
    args: &BTreeMap<String, String>,
    control: &Path,
    lease: &Lease,
) -> Result<(), String> {
    let status = HatActionStatus {
        schema: ACTION_STATUS_SCHEMA.into(),
        invocation_id: lease.record.invocation.invocation_id.clone(),
        context_partition_id: lease
            .record
            .invocation
            .context_partition
            .context_partition_id
            .clone(),
        state_revision: lease.record.status.state_revision.saturating_add(1),
        phase: HatInvocationPhase::Unresolved,
        reason_id: None,
    };
    let bytes = serde_json::to_vec(&status).map_err(message)?;
    let status_path = document_path(control, "status", &bytes);
    write_document(&status_path, &bytes)?;
    let status_common = status_common();
    let result = run_hatter(
        args,
        "worker-status",
        &status_common,
        &[
            "--worker-id",
            required(args, "worker-id")?,
            "--status-json",
            path(&status_path)?,
        ],
    );
    remove_transient(&status_path, result)
}

fn status_common() -> [String; 2] {
    ["--repository-id".to_owned(), REPOSITORY_ID.to_owned()]
}

fn process(
    args: &BTreeMap<String, String>,
    common: &[String],
    control: &Path,
    lease: &Lease,
    storage_gate: &RwLock<()>,
) -> Result<String, String> {
    let invocation = &lease.record.invocation;
    if operation(&invocation.operation_id).is_none()
        || invocation.input.owner_id != "zixcel-graph"
        || invocation.input.schema_id != hat_local_inference_manager::INPUT_SCHEMA
    {
        return Err("invocation is outside the local-inference worker contract".into());
    }
    let input_store = canonical_directory(required(args, "input-store")?)?;
    let bytes = read_digest_document(&input_store, &invocation.input)?;
    let request: ActionRequest = serde_json::from_slice(&bytes).map_err(message)?;
    if request.revision != invocation.expected_projection_revision {
        return Err("input revision differs from the invocation".into());
    }
    let catalog_state = canonical_directory(required(args, "catalog-state")?)?;
    let model_root = canonical_directory(required(args, "model-root")?)?;
    let artifact_store = canonical_directory(required(args, "artifact-store")?)?;
    let storage = Storage {
        catalog_state: &catalog_state,
        model_root: &model_root,
        artifact_store: &artifact_store,
    };
    let receipt = if mutates_state(&request.action) {
        let _guard = storage_gate
            .write()
            .map_err(|_| "local-inference mutation gate is poisoned")?;
        execute(&invocation.operation_id, request, &storage)
    } else {
        let _guard = storage_gate
            .read()
            .map_err(|_| "local-inference observation gate is poisoned")?;
        execute(&invocation.operation_id, request, &storage)
    }
    .map_err(str::to_owned)?;
    let output_bytes = serde_json::to_vec(&receipt).map_err(message)?;
    let digest = hex::encode(Sha256::digest(&output_bytes));
    let output_store = canonical_directory(required(args, "output-store")?)?;
    write_document(&output_store.join(format!("{digest}.json")), &output_bytes)?;
    let result = HatActionResult {
        schema: ACTION_RESULT_SCHEMA.into(),
        invocation_id: invocation.invocation_id.clone(),
        operation_id: invocation.operation_id.clone(),
        state_revision: lease.record.status.state_revision.saturating_add(1),
        projection_revision: receipt.revision,
        outcome: HatInvocationOutcome::Completed,
        output: Some(ActionReference {
            owner_id: REPOSITORY_ID.into(),
            reference: digest.clone(),
            schema_id: OUTPUT_SCHEMA.into(),
            digest_sha256: digest.clone(),
        }),
        reason_id: None,
        evidence_refs: vec![],
    };
    let result_bytes = serde_json::to_vec(&result).map_err(message)?;
    let result_path = document_path(control, "result", &result_bytes);
    write_document(&result_path, &result_bytes)?;
    let command = run_hatter(
        args,
        "complete",
        common,
        &[
            "--worker-id",
            required(args, "worker-id")?,
            "--result-json",
            path(&result_path)?,
        ],
    );
    remove_transient(&result_path, command)?;
    Ok(digest)
}

fn mutates_state(action: &Action) -> bool {
    matches!(
        action,
        Action::Install { .. } | Action::Remove { .. } | Action::RegisterRuntime { .. }
    )
}

fn document_path(control: &Path, kind: &str, bytes: &[u8]) -> PathBuf {
    let digest = hex::encode(Sha256::digest(bytes));
    control.join(format!("{kind}-{digest}.json"))
}

fn cleanup_transient_documents(control: &Path) -> Result<(), String> {
    let mut inspected = 0;
    for entry in fs::read_dir(control).map_err(message)? {
        let entry = entry.map_err(message)?;
        inspected += 1;
        if inspected > 4_096 {
            return Err("worker state directory exceeds its entry limit".into());
        }
        let metadata = entry.file_type().map_err(message)?;
        if metadata.is_symlink() {
            return Err("worker state directory contains a symbolic link".into());
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "worker state entry is not UTF-8".to_owned())?;
        let transient = ["registration-", "result-", "status-"]
            .into_iter()
            .any(|prefix| {
                name.strip_prefix(prefix).is_some_and(|suffix| {
                    suffix.strip_suffix(".json").is_some_and(|digest| {
                        digest.len() == 64
                            && digest.bytes().all(|value| {
                                value.is_ascii_digit() || (b'a'..=b'f').contains(&value)
                            })
                    })
                })
            });
        if transient {
            if !metadata.is_file() {
                return Err("worker transient state is not a regular file".into());
            }
            fs::remove_file(entry.path()).map_err(message)?;
        }
    }
    Ok(())
}

fn remove_transient(path: &Path, result: Result<Vec<u8>, String>) -> Result<(), String> {
    remove_transient_output(path, result).map(drop)
}

fn remove_transient_output(
    path: &Path,
    result: Result<Vec<u8>, String>,
) -> Result<Vec<u8>, String> {
    let cleanup = fs::remove_file(path).map_err(message);
    match (result, cleanup) {
        (Ok(output), Ok(())) => Ok(output),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(format!("worker control document cleanup failed: {error}")),
        (Err(error), Err(cleanup)) => Err(format!("{error}; cleanup failed: {cleanup}")),
    }
}

fn read_digest_document(root: &Path, reference: &ActionReference) -> Result<Vec<u8>, String> {
    if reference.reference != reference.digest_sha256 {
        return Err("input reference is not content-addressed".into());
    }
    let bytes = fs::read(root.join(format!("{}.json", reference.reference))).map_err(message)?;
    if bytes.len() > 1_048_576 || hex::encode(Sha256::digest(&bytes)) != reference.digest_sha256 {
        return Err("input digest differs".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::{cleanup_transient_documents, status_common};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(1);

    #[test]
    fn startup_cleanup_removes_exact_transient_control_documents() {
        let directory = std::env::temp_dir().join(format!(
            "hat-local-inference-worker-cleanup-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).expect("temporary directory");
        let digest = "a".repeat(64);
        let transient = directory.join(format!("result-{digest}.json"));
        let registration = directory.join(format!("registration-{digest}.json"));
        std::fs::write(&transient, b"transient").expect("transient document");
        std::fs::write(&registration, b"registration").expect("registration document");
        cleanup_transient_documents(&directory).expect("cleanup");
        assert!(!transient.exists());
        assert!(!registration.exists());
        std::fs::remove_dir_all(directory).expect("remove temporary directory");
    }

    #[test]
    fn unresolved_status_uses_only_the_status_command_scope() {
        assert_eq!(
            status_common(),
            ["--repository-id", "hat-local-inference-manager"]
        );
    }
}
