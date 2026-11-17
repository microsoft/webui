// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::Path;

use microsoft_webui_client_build::{build, ClientBuildOptions};

#[tokio::test]
async fn intentional_static_and_dynamic_use_returns_a_serializable_advisory(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    fs::write(temp.path().join("index.ts"), "import { value } from './shared.ts'; globalThis.staticValue = value; globalThis.dynamicValue = () => import('./shared.ts');")?;
    fs::write(temp.path().join("shared.ts"), "export const value = 42;")?;
    let result = build(ClientBuildOptions {
        project: temp.path().to_owned(),
        entries: [("index".into(), "index.ts".into())].into(),
        out_dir: "dist".into(),
        ..Default::default()
    })
    .await?;
    assert_eq!(result.warnings.len(), 1);
    assert_eq!(result.warnings[0].code, "INEFFECTIVE_DYNAMIC_IMPORT");
    assert!(result.warnings[0].message.contains("shared.ts"));
    let serialized = serde_json::to_string(&result.warnings)?;
    assert!(!serialized.contains("\\u001b"));
    assert!(serialized.contains("INEFFECTIVE_DYNAMIC_IMPORT"));
    result.projection_manifest(Path::new("dist/projection.json"))?;
    Ok(())
}
