// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Cdylib,
    Executable,
}

pub(crate) fn find(output: &[u8], target: &str, kind: Kind) -> Result<PathBuf, String> {
    let mut artifact = None;
    for line in output.split(|byte| *byte == b'\n') {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let message: Value = serde_json::from_slice(line)
            .map_err(|error| format!("Invalid Cargo artifact message: {error}"))?;
        if message["reason"] != "compiler-artifact" || message["target"]["name"] != target {
            continue;
        }
        match kind {
            Kind::Executable => {
                if let Some(file) = message["executable"].as_str() {
                    record(&mut artifact, file, target)?;
                }
            }
            Kind::Cdylib => collect_cdylib(&message, &mut artifact, target)?,
        }
    }
    artifact.ok_or_else(|| format!("Cargo did not report {target}; rebuild its host artifact"))
}

fn collect_cdylib(
    message: &Value,
    artifact: &mut Option<PathBuf>,
    target: &str,
) -> Result<(), String> {
    if !message["target"]["crate_types"]
        .as_array()
        .is_some_and(|types| types.iter().any(|kind| kind == "cdylib"))
    {
        return Ok(());
    }
    if let Some(files) = message["filenames"].as_array() {
        for file in files.iter().filter_map(Value::as_str) {
            if matches!(
                Path::new(file).extension().and_then(|part| part.to_str()),
                Some("dll" | "dylib" | "so")
            ) {
                record(artifact, file, target)?;
            }
        }
    }
    Ok(())
}

fn record(artifact: &mut Option<PathBuf>, file: &str, target: &str) -> Result<(), String> {
    if artifact.is_some() {
        return Err(format!("Cargo reported multiple {target} artifacts"));
    }
    *artifact = Some(PathBuf::from(file));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_selection_uses_reported_path_not_target_directory_assumptions() {
        for binary in ["/custom output/webui", "C:/custom output/webui.exe"] {
            let output = format!(
                r#"{{"reason":"compiler-artifact","target":{{"name":"webui"}},"executable":"{binary}"}}"#
            );
            assert_eq!(
                find(output.as_bytes(), "webui", Kind::Executable),
                Ok(PathBuf::from(binary))
            );
        }
    }

    #[test]
    fn missing_or_ambiguous_executables_fail_explicitly() {
        let message =
            br#"{"reason":"compiler-artifact","target":{"name":"webui"},"executable":null}"#;
        assert!(find(message, "webui", Kind::Executable).is_err());
        let duplicate =
            br#"{"reason":"compiler-artifact","target":{"name":"webui"},"executable":"one"}
{"reason":"compiler-artifact","target":{"name":"webui"},"executable":"two"}"#;
        assert!(find(duplicate, "webui", Kind::Executable).is_err());
    }
}
