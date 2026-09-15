// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;

#[tokio::test]
async fn initialization_timeout_is_actionable() -> Result<()> {
    let fixture = Fixture::new("export default async () => new Promise(() => {});")?;
    let result = ClientBuilder::start(
        &fixture.module,
        fixture.root.path(),
        &fixture.output,
        Duration::from_millis(500),
    )
    .await;
    let error = result.err().context("expected initialization failure")?;
    assert!(format!("{error:#}").contains("initialization timed out"));
    Ok(())
}

#[tokio::test]
async fn hung_build_is_fatal_and_child_is_reaped() -> Result<()> {
    let fixture = Fixture::new(
        "export default async () => ({ async rebuild(){ await new Promise(() => {}); }, async dispose(){} });",
    )?;
    let mut builder = fixture.start().await?;
    builder.timeout = Duration::from_millis(100);
    let result = builder.rebuild().await;
    assert!(
        matches!(result, Err(BuildError::Runtime(ref error)) if error.to_string().contains("timed out"))
    );
    assert!(worker_pid(&builder).is_none());
    builder.close().await
}

#[tokio::test]
async fn throwing_or_hung_disposal_is_fatal_and_reaped() -> Result<()> {
    for dispose in [
        "throw new Error('disposal failed')",
        "await new Promise(() => {})",
        "process.exit(7)",
        "process.exit(0)",
    ] {
        let fixture = Fixture::new(&format!(
            "export default async () => ({{ async rebuild(){{}}, async dispose(){{ {dispose}; }} }});"
        ))?;
        let mut builder = fixture.start().await?;
        builder.timeout = Duration::from_millis(100);
        assert!(builder.close().await.is_err());
        assert!(worker_pid(&builder).is_none());
        builder.close().await?;
    }
    Ok(())
}

#[tokio::test]
async fn explicit_disposal_uses_the_configured_timeout_not_the_eof_grace() -> Result<()> {
    let fixture = Fixture::new(
        "export default async () => ({ async rebuild(){}, async dispose(){ await new Promise(resolve => setTimeout(resolve, 2200)); } });",
    )?;
    let mut builder = fixture.start().await?;
    builder.close().await?;
    assert!(worker_pid(&builder).is_none());
    Ok(())
}

#[tokio::test]
async fn cancelled_build_cannot_consume_a_stale_reply() -> Result<()> {
    let fixture = Fixture::new(
        "export default async () => ({ async rebuild(){ await new Promise(() => {}); }, async dispose(){} });",
    )?;
    let mut builder = fixture.start().await?;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), builder.rebuild())
            .await
            .is_err()
    );
    assert!(matches!(
        builder.rebuild().await,
        Err(BuildError::Runtime(_))
    ));
    builder.close().await?;
    assert!(worker_pid(&builder).is_none());
    Ok(())
}

#[tokio::test]
async fn parent_eof_disposes_even_during_a_pending_build() -> Result<()> {
    for building in [false, true] {
        let fixture = Fixture::new(
            r#"
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
export default async ({ outDir }) => ({
  async rebuild() { await new Promise(() => {}); },
  async dispose() { await writeFile(join(outDir, "disposed"), "done"); },
});
"#,
        )?;
        let mut child = fixture.spawn()?;
        let mut output = BufReader::new(child.stdout.take().context("stdout")?);
        read_record(&mut output, &mut Vec::new()).await?;
        if building {
            child
                .stdin
                .as_mut()
                .context("stdin")?
                .write_all(b"{\"type\":\"build\"}\n")
                .await?;
        }
        child.stdin.take();
        assert!(tokio::time::timeout(TIMEOUT, child.wait())
            .await??
            .success());
        assert_eq!(fs::read_to_string(fixture.output.join("disposed"))?, "done");
    }
    Ok(())
}

#[tokio::test]
async fn parent_eof_bounds_hung_disposal_and_initialization() -> Result<()> {
    for source in [
        "export default async () => ({ async rebuild(){}, async dispose(){ await new Promise(() => {}); } });",
        "export default async () => new Promise(() => {});",
    ] {
        let fixture = Fixture::new(source)?;
        let mut child = fixture.spawn()?;
        child.stdin.take();
        assert!(!tokio::time::timeout(TIMEOUT, child.wait()).await??.success());
    }
    Ok(())
}

#[tokio::test]
async fn parent_eof_also_bounds_an_in_progress_stop() -> Result<()> {
    let fixture = Fixture::new(
        "export default async () => ({ async rebuild(){}, async dispose(){ await new Promise(() => {}); } });",
    )?;
    let mut child = fixture.spawn()?;
    let mut output = BufReader::new(child.stdout.take().context("stdout")?);
    read_record(&mut output, &mut Vec::new()).await?;
    child
        .stdin
        .as_mut()
        .context("stdin")?
        .write_all(b"{\"type\":\"stop\"}\n")
        .await?;
    child.stdin.take();
    assert!(!tokio::time::timeout(TIMEOUT, child.wait())
        .await??
        .success());
    Ok(())
}

#[tokio::test]
async fn dropping_builder_kills_the_worker() -> Result<()> {
    let fixture =
        Fixture::new("export default async () => ({ async rebuild(){}, async dispose(){} });")?;
    let builder = fixture.start().await?;
    let pid = worker_pid(&builder).context("worker pid")?;
    drop(builder);
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
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await??;
    Ok(())
}
