// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use tempfile::TempDir;

pub struct Fixture {
    _root: TempDir,
    pub source: PathBuf,
    pub dist: PathBuf,
    pub module: PathBuf,
    pub extra_file: PathBuf,
    pub extra_dir: PathBuf,
}

impl Fixture {
    pub fn new() -> Result<Self> {
        // Keep fixtures out of target: its globally ignored name must not mask inputs.
        let root = tempfile::Builder::new()
            .prefix("native-client-builder with spaces-")
            .tempdir_in(std::env::current_dir()?)?;
        let source = root.path().join("source with spaces");
        let dist = source.join("client output");
        let module = root.path().join("builder with spaces.mjs");
        let extra_file = root.path().join("extra-input.json");
        let extra_dir = root.path().join("shared-inputs");
        fs::create_dir_all(&dist)?;
        fs::create_dir_all(&extra_dir)?;
        fs::write(&extra_file, b"{}")?;
        fs::write(extra_dir.join("shared.ts"), b"export const shared = 1;")?;
        fs::create_dir_all(source.join("test-card"))?;
        fs::write(
            source.join("test-card").join("test-card.html"),
            "<article><h2>{{title}}</h2><if condition=\"visible\"><p>Visible</p></if></article>",
        )?;
        fs::write(
            source.join("test-card").join("test-card.css"),
            "article { color: var(--brand); }",
        )?;
        fs::write(&module, include_str!("builder.mjs"))?;
        let fixture = Self {
            _root: root,
            source,
            dist,
            module,
            extra_file,
            extra_dir,
        };
        fixture.write_source("original-source")?;
        fixture.configure(json!({}))?;
        Ok(fixture)
    }

    pub fn write_source(&self, marker: &str) -> Result<()> {
        fs::write(
            self.source.join("index.html"),
            format!(
                "<!doctype html><html><head><title>{{{{title}}}}</title>\
                 <style>:root {{ /*{{{{{{tokens.light}}}}}}*/ }}\
                 [data-dark] {{ /*{{{{{{tokens.dark}}}}}}*/ }}</style></head>\
                 <body><h1>{{{{title}}}}</h1><p>{marker}</p>\
                 <p>{{{{tokens.custom}}}}</p><test-card></test-card></body></html>"
            ),
        )?;
        Ok(())
    }

    pub fn configure(&self, overrides: Value) -> Result<()> {
        let mut input = json!({
            "title": "initial", "brand": "#112233", "dark": false, "delayMs": 1500
        });
        for (key, value) in overrides.as_object().context("fixture input object")? {
            input[key] = value.clone();
        }
        fs::write(self.source.join("input.json"), serde_json::to_vec(&input)?)?;
        Ok(())
    }

    pub fn events(&self) -> Result<Vec<Value>> {
        let path = self.dist.join("calls.ndjson");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let text = fs::read_to_string(path)?;
        text.split_inclusive('\n')
            .filter(|line| line.ends_with('\n'))
            .map(|line| serde_json::from_str(line).map_err(Into::into))
            .collect()
    }

    pub fn count(&self, kind: &str) -> Result<usize> {
        Ok(self
            .events()?
            .iter()
            .filter(|event| event["kind"] == kind)
            .count())
    }

    pub fn wait_event(&self, kind: &str, title: &str) -> Result<Value> {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let events = self.events()?;
            if let Some(event) = events
                .iter()
                .find(|event| event["kind"] == kind && event["title"] == title)
            {
                return Ok(event.clone());
            }
            if Instant::now() >= deadline {
                bail!("missing builder event {kind}/{title}; calls: {events:?}");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn assert_serial(&self) -> Result<()> {
        for event in self.events()? {
            if event["kind"] == "begin" {
                assert_eq!(event["active"], 1, "overlapping hook calls: {event}");
            }
        }
        assert_eq!(self.count("factory")?, 1, "factory must initialize once");
        Ok(())
    }

    pub fn command(&self, port: u16) -> Command {
        self.command_with_watch(port, true)
    }

    pub fn command_with_watch(&self, port: u16, watch: bool) -> Command {
        let mut command = self.client_command("serve", port);
        command.args(["--plugin", "webui"]);
        if watch {
            command.arg("--watch");
        }
        command
    }

    pub fn dev_command(&self, port: u16, no_watch: bool) -> Command {
        let mut command = self.client_command("dev", port);
        if no_watch {
            command.arg("--no-watch");
        }
        command
    }

    pub fn cli_command(&self, name: &str) -> Command {
        let executable = std::env::var_os("WEBUI_CLI_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_webui")));
        let mut command = Command::new(executable);
        command
            .arg(name)
            .env("NO_COLOR", "1")
            .env_remove("WEBUI_NO_WATCH");
        command
    }

    fn client_command(&self, name: &str, port: u16) -> Command {
        let mut command = self.cli_command(name);
        command
            .arg(&self.source)
            .args(["--entry", "index.html", "--port"])
            .arg(port.to_string())
            .arg("--servedir")
            .arg(&self.dist)
            .arg("--client-builder")
            .arg(&self.module)
            .arg("--theme")
            .arg(self.dist.join("theme.json"))
            .arg("--state")
            .arg(self.dist.join("state.json"));
        command
    }
}

pub fn bootstrap(html: &str) -> Result<Value> {
    let marker = html
        .find("id=\"webui-data\"")
        .context("SDK bootstrap metadata is absent")?;
    let tail = &html[marker..];
    let start = tail.find('>').context("SDK data tag is unclosed")? + 1;
    let end = tail[start..]
        .find("</script>")
        .context("SDK data script is unclosed")?
        + start;
    Ok(serde_json::from_str(&tail[start..end])?)
}
