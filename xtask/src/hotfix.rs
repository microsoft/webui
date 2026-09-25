// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Prepare and publish hotfix branches for supported release tags.

use crate::release_version::ReleaseVersion;
use crate::util::workspace_root;
use crate::version;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const REMOTE: &str = "origin";
const SUPPORT_PATH: &str = "xtask/src/hotfix.rs";

struct Options {
    commit: String,
    oldest: ReleaseVersion,
    dry_run: bool,
    support_commit: Option<String>,
}

struct HotfixPlan {
    base_ref: String,
    branch: String,
    version: String,
    needs_support_commit: bool,
}

pub fn run(args: &[String]) -> ExitCode {
    let options = match parse_options(args) {
        Ok(options) => options,
        Err(error) => return fail(&error),
    };
    let root = match workspace_root() {
        Ok(root) => root,
        Err(error) => return fail(&error),
    };

    match prepare_hotfixes(&root, &options) {
        Ok(count) => {
            if options.dry_run {
                eprintln!(
                    "\n  {} Planned {} hotfix branch{}\n",
                    console::style("✔").green(),
                    console::style(count).bold(),
                    if count == 1 { "" } else { "es" }
                );
            } else {
                eprintln!(
                    "\n  {} Pushed {} hotfix branch{}; Azure release builds are now queued\n",
                    console::style("✔").green(),
                    console::style(count).bold(),
                    if count == 1 { "" } else { "es" }
                );
            }
            ExitCode::SUCCESS
        }
        Err(error) => fail(&error),
    }
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut positional = Vec::with_capacity(2);
    let mut dry_run = false;
    let mut support_commit = None;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--dry-run" => dry_run = true,
            "--support-commit" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("`--support-commit` requires a commit SHA".to_string());
                };
                support_commit = Some(value.clone());
            }
            argument if argument.starts_with('-') => {
                return Err(format!(
                    "Unknown hotfix argument `{argument}`.\n  help: use \
                     `cargo xtask hotfix <commit> <oldest-tag> [--dry-run] \
                     [--support-commit <commit>]`"
                ));
            }
            argument => positional.push(argument),
        }
        index += 1;
    }

    if positional.len() != 2 {
        return Err(
            "Usage: cargo xtask hotfix <commit> <oldest-tag> [--dry-run] \
             [--support-commit <commit>]\n  \
             Example: cargo xtask hotfix abc123 v0.0.26"
                .to_string(),
        );
    }

    let oldest_value = positional[1].strip_prefix('v').unwrap_or(positional[1]);
    let oldest = ReleaseVersion::parse(oldest_value)
        .filter(|version| version.is_stable())
        .ok_or_else(|| {
            format!(
                "Invalid oldest release tag `{}`.\n  help: use a stable tag such as `v0.0.26`",
                positional[1]
            )
        })?;

    Ok(Options {
        commit: positional[0].to_string(),
        oldest,
        dry_run,
        support_commit,
    })
}

fn prepare_hotfixes(root: &Path, options: &Options) -> Result<usize, String> {
    eprintln!(
        "\n{} Preparing hotfix branches from v{} through HEAD\n",
        console::style("▸").cyan().bold(),
        console::style(options.oldest).bold()
    );

    git(root, &["fetch", REMOTE, "--tags"])?;
    let fix_commit = resolve_commit(root, &options.commit)?;
    let head = resolve_commit(root, "HEAD")?;
    let support_commit = match &options.support_commit {
        Some(commit) => resolve_commit(root, commit)?,
        None => discover_support_commit(root)?,
    };
    let tag_output = git_output(root, &["tag", "--merged", &head, "--list", "v*"])?;
    let releases = select_stable_releases(&tag_output, options.oldest);

    if !releases.contains(&options.oldest) {
        return Err(format!(
            "Tag v{} is not a stable release reachable from HEAD.\n  help: fetch the tag or choose \
             an ancestor release tag",
            options.oldest
        ));
    }

    let plans = build_plans(root, &releases, &fix_commit, &support_commit)?;
    if plans.is_empty() {
        return Err(format!(
            "Commit {fix_commit} is already included in every selected release.\n  help: choose a \
             commit newer than the selected release tags"
        ));
    }

    for plan in &plans {
        eprintln!(
            "  {} {} from {}{}",
            console::style("•").cyan(),
            console::style(&plan.branch).bold(),
            console::style(&plan.base_ref).dim(),
            if plan.needs_support_commit {
                " (bootstraps hotfix tooling)"
            } else {
                ""
            }
        );
    }

    if options.dry_run {
        return Ok(plans.len());
    }

    let mut pushed = Vec::with_capacity(plans.len());
    for plan in &plans {
        if let Err(error) = apply_plan(root, plan, &support_commit, &fix_commit) {
            if pushed.is_empty() {
                return Err(error);
            }
            return Err(format!(
                "{error}\n  already pushed before this failure: {}",
                pushed.join(", ")
            ));
        }
        pushed.push(plan.branch.as_str());
    }

    Ok(plans.len())
}

fn build_plans(
    root: &Path,
    releases: &[ReleaseVersion],
    fix_commit: &str,
    support_commit: &str,
) -> Result<Vec<HotfixPlan>, String> {
    let mut plans = Vec::with_capacity(releases.len());
    for release in releases {
        let stable_tag = format!("v{release}");
        if commit_applied(root, fix_commit, &stable_tag)? {
            eprintln!(
                "  {} {stable_tag} already contains {}",
                console::style("↷").dim(),
                short_commit(fix_commit)
            );
            continue;
        }
        if let Some(plan) = build_plan(root, *release, fix_commit, support_commit)? {
            plans.push(plan);
        }
    }
    Ok(plans)
}

fn build_plan(
    root: &Path,
    release: ReleaseVersion,
    fix_commit: &str,
    support_commit: &str,
) -> Result<Option<HotfixPlan>, String> {
    let pattern = format!("v{release}-hotfix.*");
    let hotfix_tags = git_output(root, &["tag", "--list", &pattern])?;
    let latest = latest_hotfix(release, &hotfix_tags);
    let next_number = latest
        .and_then(ReleaseVersion::hotfix_number)
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| format!("Hotfix number overflowed for v{release}"))?;
    let hotfix = release
        .with_hotfix(next_number)
        .ok_or_else(|| format!("Could not construct hotfix version for v{release}"))?;
    let branch = format!("hotfix/v{hotfix}");
    ensure_branch_available(root, &branch)?;

    let base_ref = latest
        .map(|version| format!("v{version}"))
        .unwrap_or_else(|| format!("v{release}"));
    if commit_applied(root, fix_commit, &base_ref)? {
        eprintln!(
            "  {} {base_ref} already contains {}",
            console::style("↷").dim(),
            short_commit(fix_commit)
        );
        return Ok(None);
    }

    Ok(Some(HotfixPlan {
        needs_support_commit: !commit_applied(root, support_commit, &base_ref)?,
        base_ref,
        branch,
        version: hotfix.to_string(),
    }))
}

fn apply_plan(
    root: &Path,
    plan: &HotfixPlan,
    support_commit: &str,
    fix_commit: &str,
) -> Result<(), String> {
    let worktree = hotfix_worktree_path(root, &plan.version);
    if worktree.exists() {
        return Err(format!(
            "Hotfix worktree already exists at {}.\n  help: finish or remove that worktree, then \
             retry",
            worktree.display()
        ));
    }
    if let Some(parent) = worktree.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create {}: {error}", parent.display()))?;
    }

    let worktree_arg = path_argument(&worktree)?;
    git(
        root,
        &[
            "worktree",
            "add",
            "-b",
            &plan.branch,
            &worktree_arg,
            &plan.base_ref,
        ],
    )?;

    let result = populate_hotfix_worktree(&worktree, plan, support_commit, fix_commit);
    if let Err(error) = result {
        return Err(format!(
            "{error}\n  help: resolve the retained hotfix worktree at {}",
            worktree.display()
        ));
    }

    git(root, &["worktree", "remove", "--force", &worktree_arg]).map_err(|error| {
        format!(
            "Pushed {}, but failed to remove {}: {error}",
            plan.branch,
            worktree.display()
        )
    })?;
    git(root, &["branch", "--delete", "--force", &plan.branch]).map_err(|error| {
        format!(
            "Pushed {}, but failed to delete its local branch: {error}",
            plan.branch
        )
    })?;
    Ok(())
}

fn populate_hotfix_worktree(
    worktree: &Path,
    plan: &HotfixPlan,
    support_commit: &str,
    fix_commit: &str,
) -> Result<(), String> {
    if plan.needs_support_commit {
        git(worktree, &["cherry-pick", "-x", support_commit])?;
    }
    if fix_commit != support_commit {
        git(worktree, &["cherry-pick", "-x", fix_commit])?;
    }

    let updated = version::update_workspace(worktree, &plan.version)?;
    if updated.is_empty() {
        return Err(format!(
            "Version {} did not update any workspace files",
            plan.version
        ));
    }

    git(worktree, &["add", "--all"])?;
    let message = format!("chore: prepare {}", plan.version);
    git(worktree, &["commit", "-m", &message])?;
    git(worktree, &["push", "--set-upstream", REMOTE, &plan.branch])?;
    Ok(())
}

fn select_stable_releases(output: &str, oldest: ReleaseVersion) -> Vec<ReleaseVersion> {
    let mut releases: Vec<ReleaseVersion> = output
        .lines()
        .filter_map(ReleaseVersion::parse_tag)
        .filter(|version| version.is_stable() && *version >= oldest)
        .collect();
    releases.sort_unstable();
    releases.dedup();
    releases
}

fn latest_hotfix(base: ReleaseVersion, output: &str) -> Option<ReleaseVersion> {
    output
        .lines()
        .filter_map(ReleaseVersion::parse_tag)
        .filter(|version| !version.is_stable() && version.same_base(base))
        .max()
}

fn discover_support_commit(root: &Path) -> Result<String, String> {
    let output = git_output(
        root,
        &[
            "log",
            "--diff-filter=A",
            "--format=%H",
            "--reverse",
            "--",
            SUPPORT_PATH,
        ],
    )?;
    output.lines().next().map(str::to_string).ok_or_else(|| {
        "Hotfix tooling is not committed on this branch.\n  help: run this command from a branch \
         containing the merged hotfix-support change"
            .to_string()
    })
}

fn resolve_commit(root: &Path, value: &str) -> Result<String, String> {
    let revision = format!("{value}^{{commit}}");
    git_output(root, &["rev-parse", "--verify", &revision]).map_err(|error| {
        format!(
            "Could not resolve commit `{value}`: {error}\n  help: fetch the commit or use its full SHA"
        )
    })
}

fn ensure_branch_available(root: &Path, branch: &str) -> Result<(), String> {
    let local = format!("refs/heads/{branch}");
    let remote = format!("refs/remotes/{REMOTE}/{branch}");
    if ref_exists(root, &local)? || ref_exists(root, &remote)? {
        return Err(format!(
            "Hotfix branch `{branch}` already exists without a matching release tag.\n  help: \
             finish or delete that branch before preparing another hotfix"
        ));
    }
    Ok(())
}

fn ref_exists(root: &Path, reference: &str) -> Result<bool, String> {
    command_predicate(root, &["show-ref", "--verify", "--quiet", reference])
}

fn is_ancestor(root: &Path, ancestor: &str, descendant: &str) -> Result<bool, String> {
    command_predicate(root, &["merge-base", "--is-ancestor", ancestor, descendant])
}

fn commit_applied(root: &Path, commit: &str, target: &str) -> Result<bool, String> {
    if is_ancestor(root, commit, target)? {
        return Ok(true);
    }

    let revision = git_output(root, &["rev-list", "--parents", "--max-count=1", commit])?;
    let mut revisions = revision.split_whitespace();
    let resolved = revisions
        .next()
        .ok_or_else(|| format!("Git did not return commit metadata for {commit}"))?;
    let parent = revisions.next().ok_or_else(|| {
        format!(
            "Commit {resolved} has no parent and cannot be applied as a hotfix.\n  help: choose a \
             non-root fix commit"
        )
    })?;
    if revisions.next().is_some() {
        return Err(format!(
            "Commit {resolved} is a merge commit and cannot be applied as a hotfix.\n  help: choose \
             the individual fix commit instead"
        ));
    }

    let output = git_output(root, &["cherry", target, resolved, parent])?;
    let mut lines = output.lines().filter(|line| !line.is_empty());
    let result = match lines.next().and_then(|line| line.as_bytes().first()) {
        Some(b'-') => true,
        Some(b'+') => false,
        _ => {
            return Err(format!(
                "Git could not compare commit {resolved} with {target}.\n  help: verify both \
                 revisions and retry"
            ));
        }
    };
    if lines.next().is_some() {
        return Err(format!(
            "Git returned multiple patch comparisons for commit {resolved}"
        ));
    }
    Ok(result)
}

fn command_predicate(root: &Path, args: &[&str]) -> Result<bool, String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .map_err(|error| format!("Failed to run git: {error}"))?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(command_error(args, &output.stderr)),
    }
}

fn git(root: &Path, args: &[&str]) -> Result<(), String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .map_err(|error| format!("Failed to run git: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_error(args, &output.stderr))
    }
}

fn git_output(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .map_err(|error| format!("Failed to run git: {error}"))?;
    if !output.status.success() {
        return Err(command_error(args, &output.stderr));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim_end_matches(['\r', '\n']).to_string())
        .map_err(|error| format!("Git returned non-UTF-8 output: {error}"))
}

fn command_error(args: &[&str], stderr: &[u8]) -> String {
    let detail = String::from_utf8_lossy(stderr);
    format!("`git {}` failed: {}", args.join(" "), detail.trim())
}

fn hotfix_worktree_path(root: &Path, version: &str) -> PathBuf {
    root.join("target").join("hotfix-worktrees").join(version)
}

fn path_argument(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| format!("Path is not valid UTF-8: {}", path.display()))
}

fn short_commit(commit: &str) -> &str {
    commit.get(..12).unwrap_or(commit)
}

fn fail(error: &str) -> ExitCode {
    eprintln!("  {} {error}", console::style("✘").red().bold());
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    struct HotfixRepository {
        directory: TempDir,
        support: String,
        first_fix: String,
        second_fix: String,
    }

    impl HotfixRepository {
        fn create() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            test_git(root, &["init", "--quiet"]);
            test_git(root, &["config", "user.email", "hotfix-test@example.com"]);
            test_git(root, &["config", "user.name", "Hotfix Test"]);

            fs::write(root.join("base.txt"), "base").unwrap();
            test_git(root, &["add", "base.txt"]);
            test_git(root, &["commit", "--quiet", "-m", "base"]);
            test_git(root, &["tag", "v1.0.0"]);
            test_git(root, &["switch", "--quiet", "-c", "source"]);

            fs::write(root.join("support.txt"), "support").unwrap();
            test_git(root, &["add", "support.txt"]);
            test_git(root, &["commit", "--quiet", "-m", "add hotfix support"]);
            let support = test_git(root, &["rev-parse", "HEAD"]);

            fs::write(root.join("first-fix.txt"), "first fix").unwrap();
            test_git(root, &["add", "first-fix.txt"]);
            test_git(root, &["commit", "--quiet", "-m", "first fix"]);
            let first_fix = test_git(root, &["rev-parse", "HEAD"]);

            fs::write(root.join("second-fix.txt"), "second fix").unwrap();
            test_git(root, &["add", "second-fix.txt"]);
            test_git(root, &["commit", "--quiet", "-m", "second fix"]);
            let second_fix = test_git(root, &["rev-parse", "HEAD"]);

            test_git(root, &["switch", "--quiet", "--detach", "v1.0.0"]);
            test_git(root, &["cherry-pick", "--quiet", "-x", &support]);
            test_git(root, &["cherry-pick", "--quiet", "-x", &first_fix]);
            test_git(root, &["tag", "v1.0.0-hotfix.1"]);

            Self {
                directory,
                support,
                first_fix,
                second_fix,
            }
        }

        fn root(&self) -> &Path {
            self.directory.path()
        }
    }

    fn test_git(root: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    #[test]
    fn parses_hotfix_arguments_with_optional_v_prefix() {
        let options = parse_options(&[
            "abc123".to_string(),
            "0.0.26".to_string(),
            "--dry-run".to_string(),
        ]);

        assert!(options.is_ok());
        let options = options.unwrap();
        assert_eq!(options.commit, "abc123");
        assert_eq!(options.oldest.to_string(), "0.0.26");
        assert!(options.dry_run);
        assert!(options.support_commit.is_none());
    }

    #[test]
    fn parses_support_commit_override() {
        let options = parse_options(&[
            "abc123".to_string(),
            "v0.0.26".to_string(),
            "--support-commit".to_string(),
            "def456".to_string(),
        ])
        .unwrap();

        assert_eq!(options.support_commit.as_deref(), Some("def456"));
    }

    #[test]
    fn rejects_hotfix_tag_as_oldest_release() {
        let result = parse_options(&["abc123".to_string(), "v0.0.26-hotfix.1".to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn selects_sorted_stable_release_range() {
        let oldest = ReleaseVersion::parse("0.0.26").unwrap();
        let releases = select_stable_releases(
            "v0.0.27\nv0.0.25\nv0.0.26-hotfix.1\nv0.0.26\nv1.0.0-alpha.1\n",
            oldest,
        );

        assert_eq!(
            releases.iter().map(ToString::to_string).collect::<Vec<_>>(),
            ["0.0.26", "0.0.27"]
        );
    }

    #[test]
    fn increments_latest_hotfix_number() {
        let base = ReleaseVersion::parse("0.0.27").unwrap();
        let latest = latest_hotfix(
            base,
            "v0.0.27-hotfix.1\nv0.0.26-hotfix.99\nv0.0.27-hotfix.10\nv0.0.27-hotfix.9\n",
        );

        assert_eq!(latest.and_then(ReleaseVersion::hotfix_number), Some(10));
    }

    #[test]
    fn sequential_hotfixes_recognize_cherry_picked_commits() {
        let repository = HotfixRepository::create();
        let release = ReleaseVersion::parse("1.0.0").unwrap();

        let next = build_plan(
            repository.root(),
            release,
            &repository.second_fix,
            &repository.support,
        )
        .unwrap()
        .unwrap();
        assert_eq!(next.base_ref, "v1.0.0-hotfix.1");
        assert_eq!(next.version, "1.0.0-hotfix.2");
        assert!(!next.needs_support_commit);

        let retry = build_plan(
            repository.root(),
            release,
            &repository.first_fix,
            &repository.support,
        )
        .unwrap();
        assert!(retry.is_none());
    }

    #[test]
    fn resolve_commit_removes_git_line_ending() {
        let repository = HotfixRepository::create();

        let resolved = resolve_commit(repository.root(), "HEAD").unwrap();
        let expected = test_git(repository.root(), &["rev-parse", "HEAD"]);

        assert_eq!(resolved, expected);
        assert_eq!(resolved.len(), 40);
    }

    #[test]
    fn release_pipelines_guard_hotfix_publication() {
        let build = include_str!("../../.ado/pipelines/azure-pipelines-build.yml");
        let cd = include_str!("../../.ado/pipelines/azure-pipelines-cd.yml");
        let publish_stage = cd
            .split_once("- stage: PublishRelease")
            .map(|(_, stage)| stage);

        assert!(build.contains("- hotfix/*"));
        assert!(build.contains(r#"refs/heads/hotfix/v${release_version}"#));
        assert!(build.contains(r#": "${PYTHON_VERSION:?PYTHON_VERSION is required}""#));
        assert!(build.contains("- stage: ValidateHotfix"));
        assert!(build.contains("cargo xtask check"));
        assert!(build.contains("eq(dependencies.PrepareRelease.result, 'Succeeded')"));
        assert!(build.contains(r#"git merge-base --is-ancestor "$stable_ref" "$release_commit""#));
        assert!(!cd.contains("- hotfix/*"));
        assert!(cd.contains(r#"refs/heads/hotfix/v${release_version}"#));
        assert!(cd.contains(
            r#"git merge-base --is-ancestor "${stable_ref}^{commit}" "$release_commit""#
        ));
        assert!(publish_stage.is_some_and(|stage| {
            stage
                .contains("releasePrerelease: $[ stageDependencies.SignArtifacts.SignNuGet.outputs")
                && stage.contains(
                    "releaseMakeLatest: $[ stageDependencies.SignArtifacts.SignNuGet.outputs",
                )
                && stage.contains(
                    "releaseAddChangelog: $[ stageDependencies.SignArtifacts.SignNuGet.outputs",
                )
                && stage
                    .contains("npmDistTag: $[ stageDependencies.SignArtifacts.SignNuGet.outputs")
                && stage.contains("NPM_CONFIG_TAG: $(npmDistTag)")
                && stage.contains("addChangeLog: $(releaseAddChangelog)")
        }));
    }

    #[test]
    fn release_pipelines_parallelize_release_work() {
        let build = include_str!("../../.ado/pipelines/azure-pipelines-build.yml");
        let cd = include_str!("../../.ado/pipelines/azure-pipelines-cd.yml");

        assert!(build.contains("ComponentGovernanceComponentDetection@0"));
        assert!(build.contains("sourceScanPath: $(Build.SourcesDirectory)"));
        assert!(build.contains("- job: BuildWasmAssets"));
        assert!(build.contains("artifactName: stage-wasm"));
        assert!(build.contains("--prebuilt-wasm"));

        assert!(cd.contains("- job: SignNuGet"));
        assert!(cd.contains("- job: StageNpmAndCrates"));
        assert!(cd.contains("- job: StagePythonAndStandalone"));
        assert!(cd.contains(
            "releaseTag: $[ stageDependencies.SignArtifacts.SignNuGet.outputs['release.releaseTag'] ]"
        ));
    }
}
