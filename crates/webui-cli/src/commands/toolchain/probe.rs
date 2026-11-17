// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Bounded version probe, run only after complete artifact integrity validation.

use std::io;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;

use super::ToolchainError;

pub(super) fn check(compiler: &Path) -> Result<(), ToolchainError> {
    // resolve() is synchronous and may itself be called inside a Tokio runtime.
    // A scoped worker avoids nested-runtime panics and is always joined. Async
    // pipe cancellation closes handles even if a broken binary inherits stdout
    // into descendants; there are no unbounded blocking reader threads.
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("webui-checker-probe".to_owned())
            .spawn_scoped(scope, || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| failed(compiler, error.to_string()))?;
                runtime.block_on(run(compiler))
            })
            .map_err(|error| failed(compiler, error.to_string()))?
            .join()
            .map_err(|_| failed(compiler, "native compiler probe worker failed"))?
    })
}

async fn run(compiler: &Path) -> Result<(), ToolchainError> {
    let mut child = Command::new(compiler)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| failed(compiler, error.to_string()))?;
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("missing compiler stdout"))?;
        let read = async {
            let mut bytes = Vec::with_capacity(64);
            stdout.take(1025).read_to_end(&mut bytes).await?;
            if bytes.len() > 1024 {
                return Err(io::Error::other("compiler version output exceeds 1 KiB"));
            }
            Ok(bytes)
        };
        futures_util::future::try_join(child.wait(), read).await
    })
    .await;
    let expected = format!("Version {}", super::contract::TYPESCRIPT_VERSION);
    match result {
        Ok(Ok((status, bytes)))
            if status.success()
                && bytes
                    .strip_suffix(b"\r\n")
                    .or_else(|| bytes.strip_suffix(b"\n"))
                    == Some(expected.as_bytes()) =>
        {
            Ok(())
        }
        result => {
            // Kill and reap the directly owned process before returning on all
            // timeout, I/O, overflow and incorrect-version paths.
            if child.id().is_some() {
                let _ = child.kill().await;
            }
            let reason = match result {
                Err(_) => "compiler --version exceeded five seconds".to_owned(),
                Ok(Err(error)) => error.to_string(),
                _ => format!("expected native compiler --version to report {expected}"),
            };
            Err(failed(compiler, reason))
        }
    }
}

#[cold]
#[inline(never)]
fn failed(path: &Path, reason: impl Into<String>) -> ToolchainError {
    ToolchainError::Compiler {
        path: path.to_owned(),
        reason: reason.into(),
    }
}
