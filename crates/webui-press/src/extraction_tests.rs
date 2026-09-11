// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::process::{Child, Command};
use std::time::{Duration, Instant};

use super::*;

const TEST_ROOT_ENV: &str = "WEBUI_PRESS_EXTRACTION_TEST_ROOT";
const WORKER_ENV: &str = "WEBUI_PRESS_EXTRACTION_WORKER";
const TIMEOUT: Duration = Duration::from_secs(30);

// Only the dedicated subprocess test enables these barriers. Production builds
// omit both the barriers and their call sites.
pub(super) fn checkpoint(phase: &str) -> Result<()> {
    let Some(root) = std::env::var_os(TEST_ROOT_ENV) else {
        return Ok(());
    };
    let worker = std::env::var(WORKER_ENV)?;
    let root = PathBuf::from(root);
    fs::write(root.join(format!("{worker}-{phase}")), [])?;
    wait_for(
        || {
            Ok(root
                .join(format!("{worker}-{phase}-continue"))
                .try_exists()?)
        },
        &format!("release of {worker} at {phase}"),
    )
}

#[test]
fn extraction_worker() -> Result<()> {
    // The test harness also discovers this entry point in normal, non-worker runs.
    let Some(root) = std::env::var_os(TEST_ROOT_ENV) else {
        return Ok(());
    };
    let root = PathBuf::from(root);
    let worker = std::env::var(WORKER_ENV)?;
    let template = extract_embedded_assets_in(&root.join("cache"))?;
    fs::write(
        root.join(format!("{worker}-result.json")),
        serde_json::to_vec(&template)?,
    )?;
    Ok(())
}

#[test]
fn concurrent_cold_extractions_share_complete_cache() -> Result<()> {
    let fixture = tempfile::tempdir()?;
    let cache = fixture.path().join("cache");
    fs::create_dir(&cache)?;
    let name = format!(
        "webui-press-{}-{:016x}",
        env!("CARGO_PKG_VERSION"),
        embedded_assets_hash()
    );
    let root = cache.join(&name);
    let staging = cache.join(format!("{name}.staging"));

    let mut first = ExtractionProcess::spawn(fixture.path(), "first")?;
    first.wait_at("cold")?;
    assert!(!root.exists());
    first.resume("cold")?;
    first.wait_at("staged")?;
    assert!(staging.is_dir());
    assert!(!is_complete_cache(&root));
    assert!(!staging.join(".complete").exists());

    // Keep the first writer inside its incomplete staging tree while a separate
    // process reaches the cold path. Probe the actual OS lock, not elapsed time.
    let mut second = ExtractionProcess::spawn(fixture.path(), "second")?;
    second.wait_at("cold")?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(cache.join(format!("{name}.lock")))?;
    match lock.try_lock() {
        Err(fs::TryLockError::WouldBlock) => {}
        Err(error) => return Err(error.into()),
        Ok(()) => anyhow::bail!("cold extraction must hold the cache lock while staging"),
    }

    second.resume("cold")?;
    // If the post-lock completeness check regresses, let the second writer exit
    // so its unexpected staging checkpoint fails below instead of timing out.
    second.resume("staged")?;
    first.resume("staged")?;
    let first_template = first.finish()?;
    let second_template = second.finish()?;

    assert_eq!(first_template, root.join("template"));
    assert_eq!(second_template, first_template);
    assert!(is_complete_cache(&root));
    assert!(!staging.exists());
    assert!(!fixture.path().join("second-staged").exists());
    assert_embedded_files(&EMBEDDED_TEMPLATE, &first_template)?;
    assert_embedded_files(&EMBEDDED_COMPONENTS, &root.join("components"))?;
    Ok(())
}

fn assert_embedded_files(embedded: &Dir<'_>, output: &Path) -> Result<()> {
    let mut pending = vec![embedded];
    while let Some(dir) = pending.pop() {
        for entry in dir.entries() {
            match entry {
                DirEntry::Dir(child) => {
                    assert!(output.join(child.path()).is_dir());
                    pending.push(child);
                }
                DirEntry::File(file) => {
                    assert_eq!(
                        fs::read(output.join(file.path()))?,
                        file.contents(),
                        "incomplete embedded asset: {}",
                        file.path().display()
                    );
                }
            }
        }
    }
    Ok(())
}

struct ExtractionProcess {
    child: Child,
    root: PathBuf,
    name: &'static str,
}

impl ExtractionProcess {
    fn spawn(root: &Path, name: &'static str) -> Result<Self> {
        let child = Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "extraction_tests::extraction_worker",
                "--nocapture",
            ])
            .env(TEST_ROOT_ENV, root)
            .env(WORKER_ENV, name)
            .spawn()?;
        Ok(Self {
            child,
            root: root.to_path_buf(),
            name,
        })
    }

    fn wait_at(&mut self, phase: &str) -> Result<()> {
        let marker = self.root.join(format!("{}-{phase}", self.name));
        wait_for(
            || {
                if marker.try_exists()? {
                    return Ok(true);
                }
                anyhow::ensure!(
                    self.child.try_wait()?.is_none(),
                    "{} exited before reaching {phase}",
                    self.name
                );
                Ok(false)
            },
            &format!("{} to reach {phase}", self.name),
        )
    }

    fn resume(&self, phase: &str) -> Result<()> {
        fs::write(
            self.root.join(format!("{}-{phase}-continue", self.name)),
            [],
        )?;
        Ok(())
    }

    fn finish(mut self) -> Result<PathBuf> {
        wait_for(
            || match self.child.try_wait()? {
                Some(status) => {
                    anyhow::ensure!(status.success(), "{} failed: {status}", self.name);
                    Ok(true)
                }
                None => Ok(false),
            },
            &format!("{} to finish extracting", self.name),
        )?;
        Ok(serde_json::from_slice(&fs::read(
            self.root.join(format!("{}-result.json", self.name)),
        )?)?)
    }
}

impl Drop for ExtractionProcess {
    fn drop(&mut self) {
        // Always reap subprocesses, including when an assertion or barrier fails.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_for(mut ready: impl FnMut() -> Result<bool>, description: &str) -> Result<()> {
    let deadline = Instant::now() + TIMEOUT;
    while !ready()? {
        anyhow::ensure!(
            Instant::now() < deadline,
            "timed out waiting for {description}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}
