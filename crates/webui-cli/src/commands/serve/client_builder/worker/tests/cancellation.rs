// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;

const BLOCK_READY: &str = r#"
import { writeFile } from "node:fs/promises";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
export default async ({ outDir }) => {
  process.stdout._write = () => writeFileSync(join(outDir, "ready-blocked"), String(process.pid));
  return {
    async rebuild() {},
    async dispose() {
      await new Promise(resolve => setTimeout(resolve, 20));
      await writeFile(join(outDir, "disposed"), "done");
    },
  };
};
"#;

fn busy_builder(hung_dispose: bool) -> Result<Fixture> {
    Fixture::new(&format!(
        r#"
import {{ writeFile }} from "node:fs/promises";
import {{ join }} from "node:path";
export default async ({{ outDir }}) => ({{
  async rebuild() {{ await new Promise(() => {{}}); }},
  async dispose() {{
    await new Promise(resolve => setTimeout(resolve, 20));
    await writeFile(join(outDir, "disposed"), "done");
    if ({hung_dispose}) await new Promise(() => {{}});
  }},
}});
"#
    ))
}

async fn wait_for_marker(path: &Path) -> Result<()> {
    tokio::time::timeout(TIMEOUT, async {
        while !path.is_file() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .context("Timed out waiting for disposal marker")
}

async fn wait_for_exit(pid: u32) -> Result<()> {
    tokio::time::timeout(TIMEOUT, async {
        loop {
            let status = Command::new("node")
                .args([
                    "--eval",
                    "try { process.kill(Number(process.argv[1]), 0); process.exit(1); } catch { process.exit(0); }",
                    "--",
                ])
                .arg(pid.to_string())
                .status()
                .await?;
            if status.success() {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await?
}

#[tokio::test]
async fn closing_interrupted_build_disposes_before_reaping() -> Result<()> {
    let fixture = busy_builder(false)?;
    let mut builder = fixture.start().await?;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), builder.rebuild())
            .await
            .is_err()
    );
    builder.close().await?;
    assert_eq!(fs::read_to_string(fixture.output.join("disposed"))?, "done");
    assert!(worker_pid(&builder).is_none());
    Ok(())
}

#[tokio::test]
async fn closing_interrupted_build_kills_hung_disposal() -> Result<()> {
    let fixture = busy_builder(true)?;
    let mut builder = fixture.start().await?;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), builder.rebuild())
            .await
            .is_err()
    );
    builder.timeout = Duration::from_millis(150);
    assert!(tokio::time::timeout(TIMEOUT, builder.close())
        .await?
        .is_err());
    assert_eq!(fs::read_to_string(fixture.output.join("disposed"))?, "done");
    assert!(worker_pid(&builder).is_none());
    Ok(())
}

#[tokio::test]
async fn initialization_timeout_disposes_already_returned_resources() -> Result<()> {
    let fixture = Fixture::new(BLOCK_READY)?;
    let result = ClientBuilder::start(
        &fixture.module,
        fixture.root.path(),
        &fixture.output,
        Duration::from_secs(1),
    )
    .await;
    let error = result.err().context("Expected initialization timeout")?;
    assert!(format!("{error:#}").contains("initialization timed out"));
    assert_eq!(fs::read_to_string(fixture.output.join("disposed"))?, "done");
    let pid = fs::read_to_string(fixture.output.join("ready-blocked"))?.parse()?;
    wait_for_exit(pid).await
}

#[tokio::test]
async fn initialization_cancellation_disposes_already_returned_resources() -> Result<()> {
    let fixture = Fixture::new(BLOCK_READY)?;
    let mut startup = Box::pin(fixture.start());
    let marker = fixture.output.join("ready-blocked");
    tokio::select! {
        _ = &mut startup => bail!("Initialization completed despite blocked readiness"),
        result = wait_for_marker(&marker) => result?,
    }
    let pid = fs::read_to_string(marker)?.parse()?;
    drop(startup);
    wait_for_marker(&fixture.output.join("disposed")).await?;
    wait_for_exit(pid).await
}

#[tokio::test]
async fn dropped_interrupted_build_attempts_disposal_and_bounds_cleanup() -> Result<()> {
    for hung_dispose in [false, true] {
        let fixture = busy_builder(hung_dispose)?;
        let mut builder = fixture.start().await?;
        assert!(
            tokio::time::timeout(Duration::from_millis(50), builder.rebuild())
                .await
                .is_err()
        );
        builder.timeout = Duration::from_millis(200);
        let pid = worker_pid(&builder).context("Worker pid")?;
        drop(builder);
        wait_for_marker(&fixture.output.join("disposed")).await?;
        wait_for_exit(pid).await?;
    }
    Ok(())
}

#[tokio::test]
async fn owned_worker_can_close_after_initialization_is_cancelled() -> Result<()> {
    let fixture = Fixture::new(BLOCK_READY)?;
    let mut builder = ClientBuilder::spawn(
        &fixture.module,
        fixture.root.path(),
        &fixture.output,
        TIMEOUT,
    )?;
    let marker = fixture.output.join("ready-blocked");
    {
        let initialization = builder.initialize();
        tokio::pin!(initialization);
        tokio::select! {
            _ = &mut initialization => bail!("Initialization completed despite blocked readiness"),
            result = wait_for_marker(&marker) => result?,
        }
    }
    builder.close().await?;
    assert_eq!(fs::read_to_string(fixture.output.join("disposed"))?, "done");
    assert!(worker_pid(&builder).is_none());
    Ok(())
}

#[tokio::test]
async fn spawned_worker_can_close_before_initialize_is_polled() -> Result<()> {
    let fixture = Fixture::new(BLOCK_READY)?;
    let mut builder = ClientBuilder::spawn(
        &fixture.module,
        fixture.root.path(),
        &fixture.output,
        TIMEOUT,
    )?;
    wait_for_marker(&fixture.output.join("ready-blocked")).await?;
    builder.close().await?;
    assert_eq!(fs::read_to_string(fixture.output.join("disposed"))?, "done");
    assert!(worker_pid(&builder).is_none());
    Ok(())
}

#[tokio::test]
async fn interrupted_close_drains_pipe_backpressure_before_disposal() -> Result<()> {
    let fixture = Fixture::new(
        r#"
import { existsSync, writeFileSync } from "node:fs";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { Writable } from "node:stream";
export default async ({ outDir }) => {
  // Preserve Node's nonblocking pipe handling while bypassing log redirection.
  const rawWrite = Writable.prototype.write.bind(process.stdout);
  let finish;
  const flushed = new Promise((resolve, reject) => {
    finish = error => error ? reject(error) : resolve();
  });
  return {
    async rebuild() {
      await writeFile(join(outDir, "begin"), "begin");
      while (!existsSync(join(outDir, "release"))) {
        await new Promise(resolve => setTimeout(resolve, 10));
      }
      // stdout may write synchronously; let the parent enter drainage first.
      writeFileSync(join(outDir, "flooded"), "yes");
      rawWrite(Buffer.alloc(1024 * 1024, 120), finish);
      await flushed;
    },
    async dispose() {
      await flushed;
      await writeFile(join(outDir, "disposed"), "done");
    },
  };
};
"#,
    )?;
    let mut builder = fixture.start().await?;
    let begin = fixture.output.join("begin");
    {
        let rebuild = builder.rebuild();
        tokio::pin!(rebuild);
        tokio::select! {
            _ = &mut rebuild => bail!("Build completed before its release signal"),
            result = wait_for_marker(&begin) => result?,
        }
    }
    fs::write(fixture.output.join("release"), "release")?;
    wait_for_marker(&fixture.output.join("flooded")).await?;
    assert_eq!(fs::read_to_string(fixture.output.join("flooded"))?, "yes");
    builder.close().await?;
    assert_eq!(fs::read_to_string(fixture.output.join("disposed"))?, "done");
    assert!(worker_pid(&builder).is_none());
    Ok(())
}
