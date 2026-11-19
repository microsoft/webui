// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use serde_yaml::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

fn pipeline() -> Value {
    serde_yaml::from_str(include_str!(
        "../../.ado/pipelines/azure-pipelines-build.yml"
    ))
    .unwrap_or_else(|error| panic!("invalid release YAML: {error}"))
}

fn sequence(value: &Value) -> &[Value] {
    value.as_sequence().map(Vec::as_slice).unwrap_or_default()
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

fn build_jobs(pipeline: &Value) -> &[Value] {
    let stage = sequence(&pipeline["stages"])
        .iter()
        .find(|stage| text(&stage["stage"]) == "BuildArtifacts")
        .unwrap_or_else(|| panic!("missing BuildArtifacts stage"));
    sequence(&stage["jobs"])
}

fn stage_publish(job: &Value) -> &Value {
    sequence(&job["steps"])
        .iter()
        .find(|step| {
            text(&step["task"]) == "PublishPipelineArtifact@1"
                || text(&step["template"]) == "templates/publish-stage-artifact.yml"
        })
        .unwrap_or_else(|| panic!("missing stage publisher"))
}

fn artifact_name(publish: &Value, matrix: &Value, attempt: u32) -> String {
    let name = if text(&publish["template"]).is_empty() {
        text(&publish["inputs"]["artifactName"]).to_owned()
    } else {
        let template: Value = serde_yaml::from_str(include_str!(
            "../../.ado/pipelines/templates/publish-stage-artifact.yml"
        ))
        .unwrap_or_else(|error| panic!("invalid artifact template: {error}"));
        let steps = sequence(&template["steps"]);
        assert_eq!(text(&steps[0]["name"]), "stageArtifact");
        assert!(
            text(&steps[0]["pwsh"]).contains("##vso[task.setvariable variable=name;isOutput=true]")
        );
        assert_eq!(text(&steps[1]["task"]), "PublishPipelineArtifact@1");
        assert_eq!(
            text(&steps[1]["inputs"]["artifactName"]),
            "$(stageArtifact.name)"
        );
        text(&steps[0]["env"]["STAGE_ARTIFACT_NAME"]).replace(
            "${{ parameters.artifactName }}",
            text(&publish["parameters"]["artifactName"]),
        )
    };
    name.replace("$(artifactName)", text(&matrix["artifactName"]))
        .replace("$(System.JobAttempt)", &attempt.to_string())
}

#[test]
fn release_pipeline_retries_publish_distinct_artifacts_and_download_exact_producers() {
    let pipeline = pipeline();
    let jobs = build_jobs(&pipeline);
    let mut uploaded = BTreeSet::new();
    let mut outputs = BTreeMap::new();
    let mut selected = BTreeSet::new();
    for job in jobs {
        let job_name = text(&job["job"]);
        if job_name == "AssembleRelease" {
            continue;
        }
        let matrix = job["strategy"]["matrix"].as_mapping();
        let legs: Vec<(&str, &Value)> = match matrix {
            Some(matrix) => matrix
                .iter()
                .map(|(key, value)| (text(key), value))
                .collect(),
            None => vec![("", &Value::Null)],
        };
        let publish = stage_publish(job);
        for (leg, variables) in legs {
            let original = artifact_name(publish, variables, 1);
            assert!(
                uploaded.insert(original.clone()),
                "{original} already exists"
            );
            // Model the actual failure: only Linux x64 and Windows jobs rerun
            // after successfully uploading their first attempt's artifacts.
            let retried = job_name == "BuildWindowsAssets"
                || (job_name == "BuildLinuxAssets" && leg == "LinuxX64");
            let current = if retried {
                let retry = artifact_name(publish, variables, 2);
                assert!(uploaded.insert(retry.clone()), "{retry} already exists");
                retry
            } else {
                original
            };
            let prefix = if leg.is_empty() {
                String::new()
            } else {
                format!("{leg}.")
            };
            outputs.insert(
                format!("$[ dependencies.{job_name}.outputs['{prefix}stageArtifact.name'] ]"),
                current.clone(),
            );
            selected.insert(current);
        }
    }
    assert_eq!(selected.len(), 9);
    let assembly = jobs
        .iter()
        .find(|job| text(&job["job"]) == "AssembleRelease")
        .unwrap_or_else(|| panic!("missing assembly job"));
    let mut downloaded = BTreeSet::new();
    for step in sequence(&assembly["steps"]) {
        if text(&step["task"]) != "DownloadPipelineArtifact@2" {
            continue;
        }
        let name = text(&step["inputs"]["artifactName"]);
        let variable = name
            .strip_prefix("$(")
            .and_then(|name| name.strip_suffix(')'))
            .unwrap_or_else(|| panic!("download must select its producer's output: {name}"));
        let expression = text(&assembly["variables"][variable]);
        let artifact = outputs
            .get(expression)
            .unwrap_or_else(|| panic!("unresolved artifact dependency: {expression}"));
        assert!(uploaded.contains(artifact));
        assert!(downloaded.insert(artifact.clone()));
    }
    assert_eq!(downloaded, selected);
}

#[test]
fn release_pipeline_portable_linux_does_not_archive_the_full_build_tree() {
    let pipeline = pipeline();
    let job = build_jobs(&pipeline)
        .iter()
        .find(|job| text(&job["job"]) == "BuildLinuxAssets")
        .unwrap_or_else(|| panic!("missing portable Linux job"));
    let caches: Vec<&str> = sequence(&job["steps"])
        .iter()
        .filter(|step| text(&step["task"]) == "Cache@2")
        .map(|step| text(&step["inputs"]["path"]))
        .collect();
    assert!(caches.contains(&"$(CARGO_CACHE_HOME)"));
    assert!(
        !caches.contains(&"$(Build.SourcesDirectory)/target"),
        "archiving the entire portable build tree exhausted the release runner's disk"
    );
}

#[test]
fn release_pipeline_windows_cache_restore_and_save_use_verified_gnu_tar() {
    let pipeline = pipeline();
    let job = build_jobs(&pipeline)
        .iter()
        .find(|job| text(&job["job"]) == "BuildWindowsAssets")
        .unwrap_or_else(|| panic!("missing Windows job"));
    assert!(text(&job["variables"]["cacheTar"]).ends_with(r"\Git\usr\bin\tar.exe"));
    let steps = sequence(&job["steps"]);
    let verification = steps
        .iter()
        .position(|step| text(&step["displayName"]) == "Verify GNU tar for release caches")
        .unwrap_or_else(|| panic!("missing cache archiver verification"));
    assert!(text(&steps[verification]["pwsh"]).contains("--version"));
    assert!(text(&steps[verification]["pwsh"]).contains("GNU tar"));
    assert_eq!(
        text(&steps[verification]["env"]["VSTS_TAR_EXECUTABLE"]),
        "$(cacheTar)"
    );
    let mut cache_count = 0;
    for (index, step) in steps.iter().enumerate() {
        if text(&step["task"]) == "Cache@2" {
            assert!(verification < index);
            assert_eq!(
                text(&step["env"]["VSTS_TAR_EXECUTABLE"]),
                "$(cacheTar)",
                "the agent cache plugin requires a task environment override"
            );
            assert_ne!(step["continueOnError"], Value::Bool(true));
            cache_count += 1;
        }
    }
    assert_eq!(cache_count, 2);
}

#[test]
fn release_pipeline_rejects_missing_or_malformed_outputs_before_downloading() {
    let pipeline = pipeline();
    let assembly = build_jobs(&pipeline)
        .iter()
        .find(|job| text(&job["job"]) == "AssembleRelease")
        .unwrap_or_else(|| panic!("missing assembly job"));
    let steps = sequence(&assembly["steps"]);
    let validation = steps
        .iter()
        .position(|step| text(&step["displayName"]) == "Validate staging artifact outputs")
        .unwrap_or_else(|| panic!("missing producer output validation"));
    let download_names: BTreeSet<_> = steps
        .iter()
        .enumerate()
        .filter(|(_, step)| text(&step["task"]) == "DownloadPipelineArtifact@2")
        .map(|(index, step)| {
            assert!(validation < index);
            text(&step["inputs"]["artifactName"])
        })
        .collect();
    let validation = &steps[validation];
    let configured: BTreeMap<String, String> =
        serde_json::from_str(text(&validation["env"]["STAGE_ARTIFACTS"]))
            .unwrap_or_else(|error| panic!("invalid artifact validation input: {error}"));
    assert_eq!(
        configured
            .values()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        download_names
    );
    let valid: BTreeMap<_, _> = configured
        .keys()
        .map(|prefix| (prefix.clone(), format!("{prefix}-attempt-2")))
        .collect();
    let run = |artifacts: &BTreeMap<String, String>| {
        Command::new(if cfg!(windows) { "python" } else { "python3" })
            .args(["-c", text(&validation["inputs"]["script"])])
            .env(
                "STAGE_ARTIFACTS",
                serde_json::to_string(artifacts)
                    .unwrap_or_else(|error| panic!("invalid test artifacts: {error}")),
            )
            .output()
            .unwrap_or_else(|error| panic!("cannot run pipeline output validation: {error}"))
    };
    assert!(run(&valid).status.success());
    for prefix in configured.keys() {
        for invalid in [
            String::new(),
            "$(unresolvedOutput)".to_owned(),
            format!("{prefix}-attempt-0"),
            format!("{prefix}-attempt-2/stale"),
            "unrelated-attempt-2".to_owned(),
        ] {
            let mut artifacts = valid.clone();
            artifacts.insert(prefix.clone(), invalid);
            let result = run(&artifacts);
            assert!(!result.status.success());
            assert!(String::from_utf8_lossy(&result.stderr)
                .contains(&format!("Missing or invalid producer output for {prefix}")));
        }
    }
}
