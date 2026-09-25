// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Portable guard for the Windows bootstrap; browser behavior is covered by the
// native fixture on Windows in both source and packaged modes.
#[test]
fn windows_resources_have_no_javascript_transport() -> std::io::Result<()> {
    let windows = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/windows");
    for entry in std::fs::read_dir(windows)? {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "rs") {
            let source = std::fs::read_to_string(&path)?;
            for forbidden in [
                "window.fetch =",
                "globalThis.fetch =",
                "bodyBase64",
                "FETCH_BRIDGE_SCRIPT",
                "FETCH_BRIDGE_KIND",
            ] {
                assert!(
                    !source.contains(forbidden),
                    "{}: {forbidden}",
                    path.display()
                );
            }
        }
    }
    Ok(())
}

#[test]
fn windows_requires_native_interception_for_all_request_sources() {
    let source = include_str!("../src/windows/protocol.rs");
    assert!(source.contains("AddWebResourceRequestedFilterWithRequestSourceKinds("));
    assert!(source.contains("COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL"));
    assert!(!source.contains(".AddWebResourceRequestedFilter("));
    assert!(source.contains("122.0.2365.46"));
    assert!(source.contains("CreateWebResourceResponse("));
    let backend = include_str!("../src/windows/mod.rs");
    assert!(backend.contains("w!(\"https://app.webui.localhost/*\")"));
}
