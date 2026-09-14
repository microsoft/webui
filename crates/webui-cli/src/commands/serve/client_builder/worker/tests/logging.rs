// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use tokio::io::AsyncReadExt;

use super::*;

#[tokio::test]
async fn all_console_methods_and_stdout_writes_remain_diagnostics() -> Result<()> {
    let fixture = Fixture::new(
        r#"
import { Console } from "node:console";
console.dir({ moduleDiagnostic: "module directory log" });
console.table([{ stage: "module table log" }]);
console.count("module counter");
process.stdout.write("direct module stdout\n");
export default async () => {
  console.time("factory timer");
  console.timeEnd("factory timer");
  return {
    async rebuild() {
      console.dir({ buildDiagnostic: "build directory log" });
      console.table([{ stage: "build table log" }]);
      console.count("build counter");
      console.time("build timer");
      console.timeLog("build timer", "timer progress");
      console.timeEnd("build timer");
      console.group("plugin group");
      console.log("group log");
      console.groupEnd();
      new Console(process.stdout, process.stderr).log("custom console log");
      const accepted = process.stdout.write(Buffer.from("direct buffer stdout\n"));
      if (typeof accepted !== "boolean") throw new Error("write() must return its backpressure flag");
      await new Promise(resolve => process.stdout.write("direct encoded stdout\n", "utf8", () => {
        console.info("stdout callback completed");
        resolve();
      }));
    },
    async dispose() {
      console.countReset("build counter");
      console.dir({ disposeDiagnostic: "dispose directory log" });
      process.stdout.write("direct dispose stdout\n");
    },
  };
};
"#,
    )?;
    let mut child = fixture.command().stderr(Stdio::piped()).spawn()?;
    let mut output = BufReader::new(child.stdout.take().context("stdout")?);
    assert!(matches!(
        read_record(&mut output, &mut Vec::new()).await?,
        Record::Ready { .. }
    ));
    child
        .stdin
        .as_mut()
        .context("stdin")?
        .write_all(b"{\"type\":\"build\"}\n")
        .await?;
    assert!(matches!(
        read_record(&mut output, &mut Vec::new()).await?,
        Record::Built {}
    ));
    child
        .stdin
        .as_mut()
        .context("stdin")?
        .write_all(b"{\"type\":\"stop\"}\n")
        .await?;
    assert!(matches!(
        read_record(&mut output, &mut Vec::new()).await?,
        Record::Stopped {}
    ));
    child.stdin.take();
    assert!(tokio::time::timeout(TIMEOUT, child.wait())
        .await??
        .success());
    assert!(output.fill_buf().await?.is_empty());
    let mut diagnostics = String::new();
    child
        .stderr
        .take()
        .context("stderr")?
        .read_to_string(&mut diagnostics)
        .await?;
    for expected in [
        "module directory log",
        "module table log",
        "module counter: 1",
        "direct module stdout",
        "factory timer:",
        "build directory log",
        "build table log",
        "build counter: 1",
        "build timer:",
        "timer progress",
        "plugin group",
        "group log",
        "custom console log",
        "direct buffer stdout",
        "direct encoded stdout",
        "stdout callback completed",
        "dispose directory log",
        "direct dispose stdout",
    ] {
        assert!(
            diagnostics.contains(expected),
            "missing diagnostic: {expected}"
        );
    }
    Ok(())
}
