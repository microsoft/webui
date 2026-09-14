// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;

#[tokio::test]
async fn malformed_private_output_is_fatal_and_reaped() -> Result<()> {
    for output in [
        r#"writeSync(1, 'not json\n')"#,
        r#"writeSync(1, '{"type":"unknown"}\n')"#,
        r#"writeSync(1, '{"type":"built","extra":1}\n')"#,
        r#"writeSync(1, '{"type":"error","message":42}\n')"#,
        r#"writeSync(1, '{"type":"error","message":""}\n')"#,
        r#"writeSync(1, '{"type":"ready","watchPaths":[]}\n')"#,
        r#"writeSync(1, '{"type":"built","type":"built"}\n')"#,
        r#"writeSync(1, Buffer.from([255, 10]))"#,
        r#"writeSync(1, "x".repeat(65537))"#,
        r#"writeSync(1, "x".repeat(65536) + "\n")"#,
    ] {
        let fixture = Fixture::new(&format!(
            "import {{ writeSync }} from 'node:fs'; export default async () => ({{ async rebuild() {{ {output}; await new Promise(() => {{}}); }}, async dispose() {{}} }});"
        ))?;
        let mut builder = fixture.start().await?;
        assert!(
            matches!(builder.rebuild().await, Err(BuildError::Runtime(_))),
            "accepted output: {output}"
        );
        assert!(builder.closed);
        assert!(worker_pid(&builder).is_none());
        assert!(matches!(
            builder.rebuild().await,
            Err(BuildError::Runtime(_))
        ));
        builder.close().await?;
    }
    Ok(())
}

#[tokio::test]
async fn oversized_initialization_and_raw_stdout_are_rejected() -> Result<()> {
    for source in [
        "import { writeSync } from 'node:fs'; writeSync(1, 'noise\\n'); export default async () => ({ async rebuild(){}, async dispose(){} });",
        "export default async () => ({ async rebuild(){}, async dispose(){}, watchPaths: ['a'.repeat(65536)] });",
    ] {
        let fixture = Fixture::new(source)?;
        assert!(fixture.start().await.is_err());
    }
    Ok(())
}

#[tokio::test]
async fn oversized_hook_diagnostic_is_a_runtime_failure() -> Result<()> {
    let fixture = Fixture::new(
        "export default async () => ({ async rebuild(){ throw 'x'.repeat(65537); }, async dispose(){} });",
    )?;
    let mut builder = fixture.start().await?;
    assert!(matches!(
        builder.rebuild().await,
        Err(BuildError::Runtime(_))
    ));
    assert!(worker_pid(&builder).is_none());
    Ok(())
}

#[tokio::test]
async fn exact_record_limit_is_accepted() -> Result<()> {
    let fixture = Fixture::new(
        r#"
import { writeSync } from "node:fs";
export default async () => ({
  async rebuild() {
    const overhead = JSON.stringify({ type: "error", message: "" }).length + 1;
    writeSync(1, JSON.stringify({ type: "error", message: "x".repeat(65536 - overhead) }) + "\n");
    await new Promise(() => {});
  },
  async dispose() {},
});
"#,
    )?;
    let mut builder = fixture.start().await?;
    assert!(matches!(builder.rebuild().await, Err(BuildError::Build(_))));
    assert!(builder.frame.capacity() <= MAX_RECORD);
    builder.terminate().await
}

#[tokio::test]
async fn worker_exit_or_truncated_record_never_becomes_build_success() -> Result<()> {
    for source in [
        "export default async () => ({ async rebuild(){ process.exit(0); }, async dispose(){} });",
        "export default async () => ({ async rebuild(){ process.exit(9); }, async dispose(){} });",
        "import { writeSync } from 'node:fs'; export default async () => ({ async rebuild(){ writeSync(1, '{'); process.exit(0); }, async dispose(){} });",
    ] {
        let fixture = Fixture::new(source)?;
        let mut builder = fixture.start().await?;
        assert!(matches!(builder.rebuild().await, Err(BuildError::Runtime(_))));
        assert!(worker_pid(&builder).is_none());
    }
    Ok(())
}

#[tokio::test]
async fn idle_monitor_detects_process_exit_and_close_reaps_exited_child() -> Result<()> {
    for code in [0, 9] {
        let fixture = Fixture::new(&format!(
            "export default async () => ({{ async rebuild(){{ setTimeout(() => process.exit({code}), 100); }}, async dispose(){{}} }});"
        ))?;
        let mut builder = fixture.start().await?;
        builder.rebuild().await?;
        let status = tokio::time::timeout(TIMEOUT, builder.exited()).await??;
        assert_eq!(status.code(), Some(code));
        assert!(builder.close().await.is_err());
        assert!(worker_pid(&builder).is_none());
        builder.close().await?;
    }
    Ok(())
}

#[tokio::test]
async fn cancelled_idle_wait_leaves_the_worker_available() -> Result<()> {
    let fixture =
        Fixture::new("export default async () => ({ async rebuild(){}, async dispose(){} });")?;
    let mut builder = fixture.start().await?;
    assert!(
        tokio::time::timeout(Duration::from_millis(20), builder.exited())
            .await
            .is_err()
    );
    builder.rebuild().await?;
    builder.close().await
}

#[tokio::test]
async fn cancelled_idle_wait_does_not_consume_private_output() -> Result<()> {
    let fixture = Fixture::new(
        r#"
import { writeSync } from "node:fs";
export default async () => {
  setTimeout(() => writeSync(1, '{"type":'), 20);
  return { async rebuild() {}, async dispose() {} };
};
"#,
    )?;
    let mut builder = fixture.start().await?;
    assert!(
        tokio::time::timeout(Duration::from_millis(100), builder.exited())
            .await
            .is_err()
    );
    assert!(builder.frame.is_empty());
    assert!(matches!(
        builder.rebuild().await,
        Err(BuildError::Runtime(_))
    ));
    assert!(worker_pid(&builder).is_none());
    Ok(())
}

#[tokio::test]
async fn fragmented_record_is_read_without_unbounded_buffering() -> Result<()> {
    let fixture = Fixture::new(
        r#"
import { writeSync } from "node:fs";
export default async () => ({
  async rebuild() {
    writeSync(1, '{"type":');
    await new Promise(resolve => setTimeout(resolve, 20));
    writeSync(1, '"error","message":"split"}\n');
    await new Promise(() => {});
  },
  async dispose() {},
});
"#,
    )?;
    let mut builder = fixture.start().await?;
    assert!(
        matches!(builder.rebuild().await, Err(BuildError::Build(ref message)) if message == "split")
    );
    assert!(builder.frame.capacity() <= MAX_RECORD);
    builder.terminate().await
}

#[tokio::test]
async fn private_commands_are_schema_validated_and_size_bounded() -> Result<()> {
    for command in [
        b"{\"type\":\"wat\"}\n".as_slice(),
        b"{\"type\":\"build\",\"extra\":true}\n",
        b"[]\n",
    ] {
        let fixture =
            Fixture::new("export default async () => ({ async rebuild(){}, async dispose(){} });")?;
        let mut child = fixture.spawn()?;
        let mut output = BufReader::new(child.stdout.take().context("stdout")?);
        assert!(matches!(
            read_record(&mut output, &mut Vec::new()).await?,
            Record::Ready { .. }
        ));
        child
            .stdin
            .as_mut()
            .context("stdin")?
            .write_all(command)
            .await?;
        assert!(!tokio::time::timeout(TIMEOUT, child.wait())
            .await??
            .success());
    }
    let fixture =
        Fixture::new("export default async () => ({ async rebuild(){}, async dispose(){} });")?;
    let mut child = fixture.spawn()?;
    let mut output = BufReader::new(child.stdout.take().context("stdout")?);
    read_record(&mut output, &mut Vec::new()).await?;
    child
        .stdin
        .as_mut()
        .context("stdin")?
        .write_all(&vec![b'x'; MAX_RECORD + 1])
        .await?;
    assert!(!tokio::time::timeout(TIMEOUT, child.wait())
        .await??
        .success());
    Ok(())
}
