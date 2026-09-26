// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::plan::{self, Plan};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) struct PackagePaths {
    pub binary: PathBuf,
    pub resources: PathBuf,
}

fn field<'a>(record: &'a Value, name: &str) -> Result<&'a Value, String> {
    record
        .get(name)
        .ok_or_else(|| format!("native IPC record is missing {name}"))
}

fn text<'a>(record: &'a Value, name: &str) -> Result<&'a str, String> {
    field(record, name)?
        .as_str()
        .ok_or_else(|| format!("native IPC record {name} must be a string"))
}

fn equals(record: &Value, name: &str, expected: &Value) -> Result<(), String> {
    if field(record, name)? != expected {
        return Err(format!("native IPC record {name} differs from {expected}"));
    }
    Ok(())
}

pub(super) fn record(output: &str, prefix: &str) -> Result<Value, String> {
    let mut records = output.lines().filter_map(|line| line.strip_prefix(prefix));
    let content = records
        .next()
        .ok_or_else(|| format!("expected exactly one {prefix} record, received none"))?;
    if records.next().is_some() {
        return Err(format!(
            "expected exactly one {prefix} record, received multiple"
        ));
    }
    serde_json::from_str(content).map_err(|error| format!("invalid {prefix} record: {error}"))
}

pub(super) fn runtime_dependencies(tree: &str) -> Result<(), String> {
    const FORBIDDEN: &[&str] = &[
        "microsoft-webui",
        "microsoft-webui-parser",
        "microsoft-webui-discovery",
        "microsoft-webui-desktop-build",
        "tokio",
        "rayon",
        "clap",
    ];
    let mut has_desktop = false;
    let mut unexpected = Vec::new();
    for line in tree.lines() {
        let Some(name) = line.split_whitespace().next() else {
            continue;
        };
        has_desktop |= name == "microsoft-webui-desktop";
        if FORBIDDEN.contains(&name) && !unexpected.contains(&name) {
            unexpected.push(name);
        }
    }
    if !has_desktop {
        return Err("runtime dependency tree did not include the desktop SDK".into());
    }
    if !unexpected.is_empty() {
        unexpected.sort_unstable();
        return Err(format!(
            "runtime-only consumer includes: {}",
            unexpected.join(", ")
        ));
    }
    Ok(())
}

pub(super) fn metadata(plan: Plan, source: &Value, runtime: &Value) -> Result<(), String> {
    equals(source, "platform", &Value::from(plan.platform))?;
    equals(source, "native_backend", &Value::from(plan.backend))?;
    equals(source, "package_target", &Value::from(plan.package_target))?;
    for name in ["application_ipc", "events", "window_controls", "source"] {
        equals(source, name, &Value::from(true))?;
    }
    let mut expected = source.clone();
    expected["source"] = Value::from(false);
    if runtime != &expected {
        return Err("runtime-only runner metadata differs from source runner".into());
    }
    Ok(())
}

pub(super) fn package(
    plan: Plan,
    package: &Value,
    runner: &Path,
    runtime_digest: &str,
    runtime_dir: &Path,
) -> Result<PackagePaths, String> {
    equals(package, "target", &Value::from(plan.package_target))?;
    let root = PathBuf::from(text(package, "root")?);
    let binary = PathBuf::from(text(package, "binary")?);
    let resources = PathBuf::from(text(package, "resources")?);
    if binary != root.join(plan.executable_dir).join(plan.executable())
        || resources != root.join(plan.resources)
    {
        return Err("packaged runner or resource layout differs from the SDK plan".into());
    }
    if plan::sha256(&binary)? != runtime_digest {
        return Err("packaged executable differs from runtime-only consumer".into());
    }
    if plan.is_windows() {
        plan::verify_companions(
            runtime_dir,
            binary.parent().ok_or("packaged executable has no parent")?,
        )?;
    }
    if !resources.join("manifest.webui-desktop.json").is_file() {
        return Err("packaged desktop manifest is missing".into());
    }
    if !runner.is_file() {
        return Err("runtime-only runner was removed before packaging completed".into());
    }
    Ok(PackagePaths { binary, resources })
}

pub(super) fn native_log(mode: &str, stderr: &str) -> Result<(), String> {
    for line in stderr.lines() {
        if line.starts_with("NATIVE_IPC_FAILURE")
            || line.starts_with("NATIVE_IPC_DIAGNOSTIC failure")
        {
            return Err(format!("{mode}: {line}"));
        }
    }
    Ok(())
}

pub(super) fn report(
    report: &Value,
    mode: &str,
    digest: &str,
    metadata: &Value,
) -> Result<(), String> {
    for (name, expected) in [
        ("scope", Value::from("native-ipc-four-flow")),
        ("status", Value::from("pass")),
        ("mode", Value::from(mode)),
        ("binary_sha256", Value::from(digest)),
        ("native_window_closed", Value::from(true)),
        ("native_backend", field(metadata, "native_backend")?.clone()),
        ("platform", field(metadata, "platform")?.clone()),
        ("saves", Value::from(5)),
        ("renderer_async_labels", Value::from(4)),
        ("changes", Value::from(4)),
        ("confirmations", Value::from(1)),
        ("startup_notifications", Value::from(2)),
        (
            "payload_sizes",
            Value::Array(vec![
                Value::from(0),
                Value::from(16_384),
                Value::from(262_144),
            ]),
        ),
        ("uint64", Value::from(u64::MAX.to_string())),
    ] {
        equals(report, name, &expected).map_err(|error| format!("{mode}: {error}"))?;
    }
    match text(report, "visibility")? {
        "visible" | "hidden" => {}
        other => return Err(format!("{mode}: unsupported document visibility {other}")),
    }
    for condition in [
        "invalid_input",
        "handler_failure_recovery",
        "cancellation_recovery",
        "unsubscribe",
        "void_rpc_completed",
        "notification_acceptance_not_completion",
        "full_document_navigation",
        "connection_close_and_recovery",
        "retired_session_rejected",
        "same_document_generation",
        "history_fresh_admission",
        "native_disconnect_before_navigation",
        "history_renderer_callbacks_verified",
    ] {
        equals(report, condition, &Value::from(true))
            .map_err(|error| format!("{mode}: {error}"))?;
    }
    match field(report, "persisted_restores")?.as_u64() {
        Some(0..=2) => Ok(()),
        _ => Err(format!("{mode}: persisted_restores must be 0, 1, or 2")),
    }
}

pub(super) fn write_result(artifacts: &Path, summary: &Value) -> Result<(), String> {
    let json =
        serde_json::to_string_pretty(summary).map_err(|error| format!("result JSON: {error}"))?;
    fs::write(artifacts.join("result.json"), format!("{json}\n"))
        .map_err(|error| format!("result.json: {error}"))?;
    println!("{json}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{native_log, record, report, runtime_dependencies};
    use serde_json::json;

    #[test]
    fn report_requires_every_native_assertion_not_just_pass_status() {
        let mut result = json!({
            "scope": "native-ipc-four-flow", "status": "pass", "mode": "source",
            "binary_sha256": "abc", "native_window_closed": true,
            "native_backend": "WKWebView", "platform": "darwin",
            "saves": 5, "renderer_async_labels": 4, "changes": 4,
            "confirmations": 1, "startup_notifications": 2,
            "payload_sizes": [0, 16384, 262144], "uint64": u64::MAX.to_string(),
            "visibility": "visible", "persisted_restores": 2,
            "invalid_input": true, "handler_failure_recovery": true,
            "cancellation_recovery": true, "unsubscribe": true,
            "void_rpc_completed": true, "notification_acceptance_not_completion": true,
            "full_document_navigation": true, "connection_close_and_recovery": true,
            "retired_session_rejected": true, "same_document_generation": true,
            "history_fresh_admission": true, "native_disconnect_before_navigation": true,
            "history_renderer_callbacks_verified": true,
        });
        let metadata = json!({"platform": "darwin", "native_backend": "WKWebView"});
        assert!(report(&result, "source", "abc", &metadata).is_ok());
        result["cancellation_recovery"] = json!(false);
        assert!(report(&result, "source", "abc", &metadata).is_err());
        result["cancellation_recovery"] = json!(true);
        result["saves"] = json!(4);
        assert!(report(&result, "source", "abc", &metadata).is_err());
    }

    #[test]
    fn rejection_cannot_be_hidden_by_a_pass_shaped_record() {
        let output = "NATIVE_IPC_RESULT {\"status\":\"pass\"}\n";
        assert!(record(output, "NATIVE_IPC_RESULT ").is_ok());
        assert!(native_log(
            "source",
            "NATIVE_IPC_DIAGNOSTIC failure rejection Error: code=transport\n"
        )
        .is_err());
        assert!(record(
            "NATIVE_IPC_RESULT {}\nNATIVE_IPC_RESULT {}\n",
            "NATIVE_IPC_RESULT "
        )
        .is_err());
        assert!(native_log(
            "source",
            "NATIVE_IPC_DIAGNOSTIC pageshow persisted: explicitly reconnecting\n"
        )
        .is_ok());
    }

    #[test]
    fn runtime_tree_rejects_tooling_dependencies() {
        let healthy =
            "microsoft-webui-desktop v0.0.29\nmicrosoft-webui-handler v0.0.29\nobjc2 v0.6\n";
        assert!(runtime_dependencies(healthy).is_ok());
        assert!(runtime_dependencies("microsoft-webui-desktop v0.0.29\nclap v4.0.0").is_err());
        assert!(runtime_dependencies("").is_err());
    }
}
