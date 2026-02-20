use anyhow::{Context, Result};
use clap::Args;
use std::path::PathBuf;
use webui_protocol::WebUIProtocol;

#[derive(Args)]
pub struct InspectArgs {
    /// Path to a protocol.bin file
    pub file: PathBuf,
}

pub fn execute(args: &InspectArgs) -> Result<()> {
    let protocol = WebUIProtocol::from_protobuf_file(&args.file)
        .with_context(|| format!("Failed to read {}", args.file.display()))?;

    let json = protocol
        .to_json_pretty()
        .context("Failed to serialize to JSON")?;

    println!("{json}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;
    use tempfile::TempDir;
    use webui_protocol::{WebUIFragment, WebUIFragmentRaw, WebUIFragmentSignal};

    #[test]
    fn test_inspect_outputs_valid_json() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            vec![
                WebUIFragment::Raw(WebUIFragmentRaw {
                    value: "Hello".to_string(),
                }),
                WebUIFragment::Signal(WebUIFragmentSignal {
                    value: "name".to_string(),
                    raw: false,
                }),
            ],
        );
        let protocol = WebUIProtocol { fragments };

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("protocol.bin");
        protocol.to_protobuf_file(&path).unwrap();

        let loaded = WebUIProtocol::from_protobuf_file(&path).unwrap();
        let json = loaded.to_json_pretty().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("fragments").is_some());
        assert!(parsed["fragments"]["index.html"].is_array());
    }

    #[test]
    fn test_inspect_missing_file() {
        let result = execute(&InspectArgs {
            file: PathBuf::from("/nonexistent/protocol.bin"),
        });
        assert!(result.is_err());
    }

    #[test]
    fn test_inspect_invalid_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bad.bin");
        fs::write(&path, b"not a protobuf").unwrap();

        let result = execute(&InspectArgs {
            file: path,
        });
        assert!(result.is_err());
    }
}
