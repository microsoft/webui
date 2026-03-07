use anyhow::{Context, Result};
use clap::Args;
use expand_tilde::expand_tilde;
use std::path::PathBuf;

#[derive(Args)]
pub struct InspectArgs {
    /// Path to a protocol.bin file
    pub file: PathBuf,
}

pub fn execute(args: &InspectArgs) -> Result<()> {
    let input_file = expand_tilde(&args.file)
        .with_context(|| format!("Failed to expand input path: {}", args.file.display()))?
        .into_owned();

    let json = webui::inspect(&input_file)
        .with_context(|| format!("Failed to inspect {}", args.file.display()))?;

    println!("{json}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;
    use tempfile::TempDir;
    use webui_protocol::{FragmentList, WebUIFragment, WebUIProtocol};

    #[test]
    fn test_inspect_outputs_valid_json() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Hello"),
                    WebUIFragment::signal("name", false),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("protocol.bin");
        protocol.to_protobuf_file(&path).unwrap();

        let loaded = WebUIProtocol::from_protobuf_file(&path).unwrap();
        let json = loaded.to_json_pretty().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("fragments").is_some());
        assert!(parsed["fragments"]["index.html"]["fragments"].is_array());
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

        let result = execute(&InspectArgs { file: path });
        assert!(result.is_err());
    }
}
