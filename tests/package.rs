use hat_local_inference_manager::{PACKAGE_JSON, operation};

#[test]
fn package_is_valid_and_every_operation_is_implemented() {
    let package: hat_specifications::HatPackage =
        serde_json::from_str(PACKAGE_JSON).expect("package JSON");
    let validation = hat_specifications::validate_package(&package);
    assert!(validation.valid, "{:?}", validation.findings);
    assert!(
        package
            .operations
            .iter()
            .all(|item| operation(&item.id).is_some())
    );
}

#[test]
fn action_must_match_exact_operation() {
    let request = serde_json::from_value(serde_json::json!({
        "schema": "hathq://hat-local-inference-manager/action-request/v1",
        "revision": 0,
        "action": { "kind": "inspect" }
    }))
    .expect("request");
    let temporary = std::env::temp_dir().join(format!(
        "hat-local-inference-manager-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&temporary).expect("temporary");
    let result = hat_local_inference_manager::execute(
        hat_local_inference_manager::PLAN_OPERATION,
        request,
        &hat_local_inference_manager::Storage {
            catalog_state: &temporary,
            model_root: &temporary,
            artifact_store: &temporary,
        },
    );
    assert_eq!(result.expect_err("mismatch"), "operation-action-mismatch");
    std::fs::remove_dir_all(temporary).expect("cleanup");
}
