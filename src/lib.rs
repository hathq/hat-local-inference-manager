#![forbid(unsafe_code)]
#![doc = "HAT adapter for the policy-neutral Zixcel local-inference lifecycle."]

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use zixcel_local_inference::{
    CatalogStore, RuntimeRouteRegistry, Selection, install_from_file, installation_statuses,
    installed_models, remove_installation,
};

pub const PACKAGE_JSON: &str = include_str!("../hat.package.json");
pub const REPOSITORY_ID: &str = "hat-local-inference-manager";
pub const INPUT_SCHEMA: &str = "hathq://hat-local-inference-manager/action-request/v1";
pub const OUTPUT_SCHEMA: &str = "hathq://hat-local-inference-manager/action-receipt/v1";

pub const INSPECT_OPERATION: &str =
    "hathq://vocabulary/action/inspect-local-inference-resources/v1";
pub const PLAN_OPERATION: &str = "hathq://vocabulary/action/prepare-local-model-acquisition/v1";
pub const INSTALL_OPERATION: &str = "hathq://vocabulary/action/install-local-model/v1";
pub const REMOVE_OPERATION: &str = "hathq://vocabulary/action/remove-local-model/v1";
pub const REGISTER_RUNTIME_OPERATION: &str =
    "hathq://vocabulary/action/register-local-inference-runtime/v1";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionRequest {
    pub schema: String,
    pub revision: u64,
    pub action: Action,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Action {
    Inspect {
        #[serde(default)]
        model_id: Option<String>,
    },
    PrepareAcquisition {
        model_id: String,
        #[serde(default)]
        format: Option<String>,
        #[serde(default)]
        runtime_id: Option<String>,
    },
    Install {
        model_id: String,
        artifact_digest_sha256: String,
        #[serde(default)]
        format: Option<String>,
        #[serde(default)]
        runtime_id: Option<String>,
    },
    Remove {
        model_id: String,
        release_id: String,
        confirmation: String,
    },
    RegisterRuntime {
        route_id: String,
        engine: String,
        protocol: String,
        endpoint: String,
        capabilities: Vec<String>,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionReceipt {
    pub schema: &'static str,
    pub operation_id: &'static str,
    pub revision: u64,
    pub result: Value,
    pub contains_local_paths: bool,
    pub contains_secret_values: bool,
}

pub struct Storage<'a> {
    pub catalog_state: &'a Path,
    pub model_root: &'a Path,
    pub artifact_store: &'a Path,
}

/// Executes one exact HAT action against Zixcel-owned state and returns a bounded projection.
///
/// # Errors
/// Returns a stable reason when the operation and action differ or Zixcel rejects the request.
pub fn execute(
    operation_id: &str,
    request: ActionRequest,
    storage: &Storage<'_>,
) -> Result<ActionReceipt, &'static str> {
    if request.schema != INPUT_SCHEMA {
        return Err("input-schema-invalid");
    }
    let expected_operation = match &request.action {
        Action::Inspect { .. } => INSPECT_OPERATION,
        Action::PrepareAcquisition { .. } => PLAN_OPERATION,
        Action::Install { .. } => INSTALL_OPERATION,
        Action::Remove { .. } => REMOVE_OPERATION,
        Action::RegisterRuntime { .. } => REGISTER_RUNTIME_OPERATION,
    };
    if operation_id != expected_operation {
        return Err("operation-action-mismatch");
    }
    let store = CatalogStore::open_existing(storage.catalog_state).map_err(|error| error.code())?;
    let runtimes =
        RuntimeRouteRegistry::open_existing(storage.catalog_state).map_err(|error| error.code())?;
    let result = match request.action {
        Action::Inspect { model_id } if operation_id == INSPECT_OPERATION => {
            inspect(&store, &runtimes, storage.model_root, model_id.as_deref())?
        }
        Action::PrepareAcquisition {
            model_id,
            format,
            runtime_id,
        } if operation_id == PLAN_OPERATION => plan(
            &store,
            &runtimes,
            storage.model_root,
            &model_id,
            format,
            runtime_id.as_deref(),
        )?,
        Action::Install {
            model_id,
            artifact_digest_sha256,
            format,
            runtime_id,
        } if operation_id == INSTALL_OPERATION => install(
            &store,
            &runtimes,
            storage,
            &model_id,
            &artifact_digest_sha256,
            format,
            runtime_id.as_deref(),
        )?,
        Action::Remove {
            model_id,
            release_id,
            confirmation,
        } if operation_id == REMOVE_OPERATION => {
            remove_installation(storage.model_root, &model_id, &release_id, &confirmation)
                .map_err(|error| error.code())?;
            json!({ "modelId": model_id, "releaseId": release_id, "state": "removed" })
        }
        Action::RegisterRuntime {
            route_id,
            engine,
            protocol,
            endpoint,
            capabilities,
        } if operation_id == REGISTER_RUNTIME_OPERATION => register_runtime(
            &runtimes,
            &route_id,
            &engine,
            &protocol,
            &endpoint,
            capabilities,
        )?,
        _ => return Err("operation-action-mismatch"),
    };
    let operation_id = operation(operation_id).ok_or("operation-unsupported")?;
    Ok(ActionReceipt {
        schema: OUTPUT_SCHEMA,
        operation_id,
        revision: request.revision.checked_add(1).ok_or("revision-overflow")?,
        result,
        contains_local_paths: false,
        contains_secret_values: false,
    })
}

fn inspect(
    store: &CatalogStore,
    runtimes: &RuntimeRouteRegistry,
    model_root: &Path,
    model_id: Option<&str>,
) -> Result<Value, &'static str> {
    let candidates = store.candidates().map_err(|error| error.code())?;
    let installed = installed_models(store, model_root)
        .map_err(|error| error.code())?
        .into_iter()
        .map(|value| {
            json!({ "modelId": value.model_id,
            "releaseId": value.release_id, "state": value.state })
        })
        .collect::<Vec<_>>();
    let routes = runtimes
        .routes()
        .map_err(|error| error.code())?
        .into_iter()
        .map(|value| {
            json!({ "id": value.id, "engine": value.engine,
            "protocol": value.protocol, "capabilities": value.capabilities,
            "enabled": value.enabled })
        })
        .collect::<Vec<_>>();
    let statuses = model_id.map_or_else(
        || Ok(Vec::new()),
        |value| {
            installation_statuses(store, model_root, value)
                .map_err(|error| error.code())
                .map(|values| {
                    values
                        .into_iter()
                        .map(|item| {
                            json!({ "modelId": item.model_id,
                    "releaseId": item.release_id, "format": item.format,
                    "state": item.state, "inferenceReady": item.inference_ready,
                    "runtimeEngines": item.runtime_engines })
                        })
                        .collect()
                })
        },
    )?;
    Ok(json!({ "candidates": candidates, "installed": installed,
        "runtimes": routes, "statuses": statuses }))
}

fn plan(
    store: &CatalogStore,
    runtimes: &RuntimeRouteRegistry,
    model_root: &Path,
    model_id: &str,
    format: Option<String>,
    runtime_id: Option<&str>,
) -> Result<Value, &'static str> {
    let value = store
        .acquisition_plan(
            model_id,
            model_root,
            &selection(format, runtime_id, runtimes)?,
        )
        .map_err(|error| error.code())?;
    Ok(
        json!({ "sourceId": value.source_id, "catalogId": value.catalog_id,
        "catalogSequence": value.catalog_sequence, "modelId": value.model_id,
        "releaseId": value.release_id, "artifactId": value.artifact_id,
        "artifactKind": value.artifact_kind, "format": value.format,
        "quantization": value.quantization, "runtimeEngines": value.runtime_engines,
        "sourceRepository": value.source_repository, "sourceRevision": value.source_revision,
        "expectedSha256": value.expected_sha256, "expectedBytes": value.expected_bytes,
        "transportOwner": value.transport_owner, "automaticExecution": value.automatic_execution,
        "state": value.state }),
    )
}

fn install(
    store: &CatalogStore,
    runtimes: &RuntimeRouteRegistry,
    storage: &Storage<'_>,
    model_id: &str,
    artifact_digest: &str,
    format: Option<String>,
    runtime_id: Option<&str>,
) -> Result<Value, &'static str> {
    if !lower_hex_32(artifact_digest) {
        return Err("artifact-reference-invalid");
    }
    let artifact = storage
        .artifact_store
        .join(format!("{artifact_digest}.bin"));
    let value = install_from_file(
        store,
        model_id,
        storage.model_root,
        &selection(format, runtime_id, runtimes)?,
        &artifact,
    )
    .map_err(|error| error.code())?;
    Ok(
        json!({ "modelId": value.model_id, "releaseId": value.release_id,
        "artifactId": value.artifact_id, "artifactKind": value.artifact_kind,
        "format": value.format, "artifactSha256": value.artifact_sha256,
        "artifactBytes": value.artifact_bytes, "runtimeEngines": value.runtime_engines,
        "state": value.state, "inferenceReady": value.inference_ready }),
    )
}

fn register_runtime(
    runtimes: &RuntimeRouteRegistry,
    route_id: &str,
    engine: &str,
    protocol: &str,
    endpoint: &str,
    capabilities: Vec<String>,
) -> Result<Value, &'static str> {
    let value = runtimes
        .add(route_id, engine, protocol, endpoint, capabilities)
        .map_err(|error| error.code())?;
    Ok(json!({ "id": value.id, "engine": value.engine,
        "protocol": value.protocol, "capabilities": value.capabilities,
        "enabled": value.enabled }))
}

fn selection(
    format: Option<String>,
    runtime_id: Option<&str>,
    runtimes: &RuntimeRouteRegistry,
) -> Result<Selection, &'static str> {
    let runtime_engine = runtime_id
        .map(|value| runtimes.route(value).map(|route| route.engine))
        .transpose()
        .map_err(|error| error.code())?;
    Ok(Selection {
        format,
        runtime_engine,
    })
}

#[must_use]
pub fn operation(value: &str) -> Option<&'static str> {
    [
        INSPECT_OPERATION,
        PLAN_OPERATION,
        INSTALL_OPERATION,
        REMOVE_OPERATION,
        REGISTER_RUNTIME_OPERATION,
    ]
    .into_iter()
    .find(|candidate| *candidate == value)
}

fn lower_hex_32(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
