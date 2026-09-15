// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;

use super::*;

mod cancellation;
mod lifecycle;
mod logging;
mod transport;

const TIMEOUT: Duration = Duration::from_secs(5);

impl ClientBuilder {
    async fn start(
        module: &Path,
        app_dir: &Path,
        out_dir: &Path,
        timeout: Duration,
    ) -> Result<Self> {
        let mut builder = Self::spawn(module, app_dir, out_dir, timeout)?;
        builder.initialize().await?;
        Ok(builder)
    }
}

fn worker_pid(builder: &ClientBuilder) -> Option<u32> {
    builder.child.as_ref().and_then(Child::id)
}

struct Fixture {
    root: tempfile::TempDir,
    module: PathBuf,
    output: PathBuf,
}

impl Fixture {
    fn new(source: &str) -> Result<Self> {
        let root = tempfile::Builder::new()
            .prefix(".client-worker-test-")
            .tempdir_in(std::env::current_dir()?)?;
        let module = root.path().join("builder.mjs");
        let output = root.path().join("out");
        fs::create_dir(&output)?;
        fs::write(
            &module,
            format!(
                "// Copyright (c) Microsoft Corporation.\n// Licensed under the MIT license.\n\n{source}"
            ),
        )?;
        Ok(Self {
            root,
            module,
            output,
        })
    }

    async fn start(&self) -> Result<ClientBuilder> {
        ClientBuilder::start(&self.module, self.root.path(), &self.output, TIMEOUT).await
    }

    fn spawn(&self) -> Result<Child> {
        Ok(self.command().spawn()?)
    }

    fn command(&self) -> Command {
        let mut command = Command::new("node");
        command
            .args(["--input-type=module", "--eval", SCRIPT, "--"])
            .arg(&self.module)
            .arg(self.root.path())
            .arg(&self.output)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        command
    }
}

#[tokio::test]
async fn factory_runs_once_and_hooks_settle_before_reply() -> Result<()> {
    let fixture = Fixture::new(
        r#"
import { writeFile, appendFile } from "node:fs/promises";
import { join } from "node:path";
export default async (options) => {
  const { appDir, outDir } = options;
  if (Object.keys(options).length !== 2) throw new Error("Custom factory options changed");
  await appendFile(join(outDir, "factory"), "once");
  let count = 0;
  return {
    async rebuild() {
      await new Promise(resolve => setTimeout(resolve, 20));
      await writeFile(join(outDir, "state.json"), JSON.stringify({ appDir, outDir, count: ++count }));
      return { ignored: "hook result is not a wire record" };
    },
    async dispose() {
      await new Promise(resolve => setTimeout(resolve, 20));
      await writeFile(join(outDir, "disposed"), "done");
    },
  };
};
"#,
    )?;
    let mut builder = fixture.start().await?;
    assert!(builder.watch_paths().is_empty());
    let pid = worker_pid(&builder);
    for count in 1..=3 {
        builder.rebuild().await?;
        let state: serde_json::Value =
            serde_json::from_slice(&fs::read(fixture.output.join("state.json"))?)?;
        assert_eq!(state["count"], count);
        assert_eq!(
            Path::new(state["appDir"].as_str().context("appDir")?),
            fixture.root.path()
        );
        assert_eq!(
            Path::new(state["outDir"].as_str().context("outDir")?),
            fixture.output
        );
        assert_eq!(worker_pid(&builder), pid);
    }
    builder.close().await?;
    builder.close().await?;
    assert_eq!(fs::read_to_string(fixture.output.join("factory"))?, "once");
    assert_eq!(fs::read_to_string(fixture.output.join("disposed"))?, "done");
    assert!(worker_pid(&builder).is_none());
    Ok(())
}

#[tokio::test]
async fn hook_failure_keeps_the_context_available_for_recovery() -> Result<()> {
    let fixture = Fixture::new(
        r#"
export default async () => {
  let count = 0;
  return {
    async rebuild() { if (++count === 2) throw new Error("fix the source"); },
    async dispose() {},
  };
};
"#,
    )?;
    let mut builder = fixture.start().await?;
    let pid = worker_pid(&builder);
    builder.rebuild().await?;
    let error = builder.rebuild().await;
    assert!(
        matches!(error, Err(BuildError::Build(ref message)) if message.contains("fix the source"))
    );
    builder.rebuild().await?;
    assert_eq!(pid, worker_pid(&builder));
    builder.close().await
}

#[tokio::test]
async fn watch_paths_are_resolved_once_against_app_dir() -> Result<()> {
    let fixture = Fixture::new(
        r#"
import { join } from "node:path";
export default async ({ appDir }) => {
  const watchPaths = ["extra", "../external.json", join(appDir, "absolute")];
  return {
    watchPaths,
    async rebuild() { watchPaths.push("too-late"); },
    async dispose() {},
  };
};
"#,
    )?;
    let mut builder = fixture.start().await?;
    let expected = vec![
        fixture.root.path().join("extra"),
        fixture
            .root
            .path()
            .parent()
            .context("parent")?
            .join("external.json"),
        fixture.root.path().join("absolute"),
    ];
    assert_eq!(builder.watch_paths(), expected);
    builder.rebuild().await?;
    assert_eq!(builder.watch_paths(), expected);
    builder.close().await
}

#[tokio::test]
async fn ordinary_console_logging_does_not_corrupt_private_output() -> Result<()> {
    let fixture = Fixture::new(
        r#"
console.log("module log");
console.info("module info");
console.debug("module debug");
export default async () => {
  console.log("factory log", { object: true });
  return {
    async rebuild() { console.info("build info"); console.debug("build debug"); },
    async dispose() { console.log("dispose log"); },
  };
};
"#,
    )?;
    let mut builder = fixture.start().await?;
    builder.rebuild().await?;
    builder.close().await
}

#[tokio::test]
async fn invalid_module_or_factory_fails_initialization() -> Result<()> {
    for source in [
        "export const notDefault = 1;",
        "export default 1;",
        "export default async () => null;",
        "export default async () => ({ async rebuild() {} });",
        "export default async () => ({ async dispose() {} });",
        "export default async () => { throw new Error('initialization failed'); };",
        "export default async () => ({ async rebuild(){}, async dispose(){}, watchPaths: 'bad' });",
        "export default async () => ({ async rebuild(){}, async dispose(){}, watchPaths: [42] });",
        "export default async () => ({ async rebuild(){}, async dispose(){}, watchPaths: [''] });",
        "export default async () => ({ async rebuild(){}, async dispose(){}, watchPaths: ['\\0'] });",
        "not valid javascript!",
    ] {
        let fixture = Fixture::new(source)?;
        let result = fixture.start().await;
        assert!(result.is_err(), "accepted invalid module: {source}");
    }
    let fixture = Fixture::new("")?;
    fs::remove_file(&fixture.module)?;
    assert!(fixture.start().await.is_err());
    Ok(())
}

#[test]
fn error_display_distinguishes_retriable_builds_and_fatal_runtime() {
    assert!(BuildError::Build("broken".into())
        .to_string()
        .contains("save an input to retry"));
    assert!(BuildError::Runtime(anyhow::anyhow!("closed"))
        .to_string()
        .contains("restart"));
}

#[tokio::test]
async fn builtin_sentinel_selects_embedded_factory_with_shared_lifecycle() -> Result<()> {
    let fixture = Fixture::new("throw new Error('custom module must not load');")?;
    let script = format!(
        r#"
async function createBuiltinBuilder(options) {{
  const {{ appDir, outDir, clientEntry }} = options;
  if (Object.keys(options).length !== 3) throw new Error("Builtin factory options changed");
  const {{ writeFile }} = await import("node:fs/promises");
  const {{ join }} = await import("node:path");
  let count = 0;
  console.log("builtin factory initialized");
  return {{
    watchPaths: ["shared"],
    async rebuild() {{
      await writeFile(join(outDir, "options.json"), JSON.stringify({{ appDir, outDir, clientEntry, count: ++count }}));
    }},
    async dispose() {{ await writeFile(join(outDir, "disposed"), "done"); }},
  }};
}}
{SCRIPT}
"#
    );
    let mut builder = ClientBuilder::launch(
        &script,
        &[
            OsStr::new("--builtin"),
            fixture.root.path().as_os_str(),
            fixture.output.as_os_str(),
            OsStr::new("client entry.ts"),
        ],
        TIMEOUT,
    )?;
    builder.initialize().await?;
    assert_eq!(builder.watch_paths(), [fixture.root.path().join("shared")]);
    for count in 1..=2 {
        builder.rebuild().await?;
        let options: serde_json::Value =
            serde_json::from_slice(&fs::read(fixture.output.join("options.json"))?)?;
        assert_eq!(options["count"], count);
        assert_eq!(
            Path::new(options["clientEntry"].as_str().context("clientEntry")?),
            fixture.root.path().join("client entry.ts")
        );
        assert_eq!(
            Path::new(options["appDir"].as_str().context("appDir")?),
            fixture.root.path()
        );
        assert_eq!(
            Path::new(options["outDir"].as_str().context("outDir")?),
            fixture.output
        );
    }
    builder.close().await?;
    assert_eq!(fs::read_to_string(fixture.output.join("disposed"))?, "done");
    Ok(())
}
