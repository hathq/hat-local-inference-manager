// Modified by the Hatter downstream project, 2026.
// Purpose: validate the exact producer package against the current operational contract.
#[test]
fn current_package_has_no_language_frames_and_uses_current_foundation() {
    let package = hat_specifications::parse_package(include_str!("../hat.package.json"))
        .expect("current package shape");
    let result = hat_specifications::validate_package(&package);
    assert!(result.valid, "{:?}", result.findings);
    assert!(
        package
            .catalog
            .dependencies
            .contains(&hat_specifications::foundation_semantic_catalog().identity)
    );
    let value = serde_json::to_value(&package.catalog).expect("catalog");
    assert!(value.get("frames").is_none());
}
