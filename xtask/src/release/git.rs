// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{required_env, validate_commit, validate_release_tag};
use crate::util::build_command;
use std::process::Output;

pub(super) enum RemoteTag {
    Missing,
    Commit(String),
}

pub(super) fn ensure_tag_from_env() -> Result<(), String> {
    let tag = required_env("RELEASE_TAG")?;
    let commit = required_env("RELEASE_COMMIT")?;
    validate_release_tag(&tag)?;
    validate_commit(&commit)?;
    ensure_commit_available(&commit)?;

    match remote_tag_commit(&tag)? {
        RemoteTag::Commit(existing) if existing == commit => {
            println!("Remote tag {tag} already points to {commit}.");
            return Ok(());
        }
        RemoteTag::Commit(existing) => {
            return Err(format!(
                "remote tag {tag} points to {existing}, not {commit}"
            ));
        }
        RemoteTag::Missing => {}
    }

    run_git(&["config", "user.name", "Azure Pipelines"])?;
    run_git(&["config", "user.email", "azure-pipelines@microsoft.com"])?;
    if git_success(&[
        "show-ref",
        "--verify",
        "--quiet",
        &format!("refs/tags/{tag}"),
    ])? {
        run_git(&["tag", "-d", &tag])?;
    }
    run_git(&["tag", "-a", &tag, &commit, "-m", &format!("Release {tag}")])?;

    let push = git_output(&["push", "origin", &format!("refs/tags/{tag}")])?;
    if push.status.success() {
        println!("Created annotated release tag {tag} at {commit}.");
        return Ok(());
    }

    eprintln!("Tag push failed; checking whether another release run created the same tag.");
    let race_ref = format!("refs/azure-release-tags/{tag}");
    run_git(&[
        "fetch",
        "--force",
        "origin",
        &format!("refs/tags/{tag}:{race_ref}"),
    ])?;
    let raced_commit = git_stdout(&["rev-parse", &format!("{race_ref}^{{}}")])?;
    validate_commit(&raced_commit)?;
    if raced_commit != commit {
        return Err(format!(
            "concurrent remote tag {tag} points to {raced_commit}, not {commit}"
        ));
    }
    println!("Concurrent release run created {tag} at the expected commit.");
    Ok(())
}

pub(super) fn remote_tag_commit(tag: &str) -> Result<RemoteTag, String> {
    let direct_ref = format!("refs/tags/{tag}");
    let peeled_ref = format!("{direct_ref}^{{}}");
    let output = git_output(&[
        "ls-remote",
        "--exit-code",
        "origin",
        &direct_ref,
        &peeled_ref,
    ])?;
    if output.status.code() == Some(2) {
        return Ok(RemoteTag::Missing);
    }
    if !output.status.success() {
        return Err(format!(
            "unable to query {tag} from origin: {}",
            output_text(&output)
        ));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("git ls-remote returned non-UTF-8 output: {error}"))?;
    let commit = find_remote_ref(&stdout, &peeled_ref)
        .or_else(|| find_remote_ref(&stdout, &direct_ref))
        .ok_or_else(|| format!("remote tag {tag} did not resolve to a commit ID"))?;
    validate_commit(commit)?;
    Ok(RemoteTag::Commit(commit.to_string()))
}

fn ensure_commit_available(commit: &str) -> Result<(), String> {
    run_git(&["fetch", "origin", "--tags"])?;
    if !git_success(&["cat-file", "-e", &format!("{commit}^{{commit}}")])? {
        run_git(&["fetch", "--no-tags", "origin", commit])?;
    }
    if !git_success(&["cat-file", "-e", &format!("{commit}^{{commit}}")])? {
        return Err("RELEASE_COMMIT does not identify a commit available from origin".to_string());
    }
    Ok(())
}

fn find_remote_ref<'a>(output: &'a str, expected_ref: &str) -> Option<&'a str> {
    output.lines().find_map(|line| {
        let (commit, reference) = line.split_once('\t')?;
        (reference == expected_ref).then_some(commit)
    })
}

fn git_success(args: &[&str]) -> Result<bool, String> {
    let output = git_output(args)?;
    Ok(output.status.success())
}

fn git_stdout(args: &[&str]) -> Result<String, String> {
    let output = git_output(args)?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            output_text(&output)
        ));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|error| format!("git {} returned non-UTF-8 output: {error}", args.join(" ")))
}

fn run_git(args: &[&str]) -> Result<(), String> {
    let output = git_output(args)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "git {} failed: {}",
            args.join(" "),
            output_text(&output)
        ))
    }
}

fn git_output(args: &[&str]) -> Result<Output, String> {
    build_command("git", args)
        .output()
        .map_err(|error| format!("failed to run git {}: {error}", args.join(" ")))
}

fn output_text(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let text = format!("{stdout}{stderr}");
    text.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_ref_prefers_exact_reference() {
        let output = concat!(
            "0123456789abcdef0123456789abcdef01234567\trefs/tags/v1.2.3\n",
            "89abcdef0123456789abcdef0123456789abcdef\trefs/tags/v1.2.3^{}\n",
        );

        assert_eq!(
            find_remote_ref(output, "refs/tags/v1.2.3^{}"),
            Some("89abcdef0123456789abcdef0123456789abcdef")
        );
    }
}
