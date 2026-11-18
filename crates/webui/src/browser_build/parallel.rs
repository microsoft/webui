// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::Path;

use rolldown::plugin::HookRenderChunkArgs;

use super::{loaders::ascii_literal, resources, ClientBuildError};

/// Preliminary output identities, established by this exact Rolldown invocation.
pub(super) fn static_closure<'a>(
    args: &'a HookRenderChunkArgs<'_>,
    start: &'a str,
) -> Result<BTreeSet<&'a str>, ClientBuildError> {
    let mut seen = BTreeSet::new();
    let mut pending = vec![start];
    while let Some(filename) = pending.pop() {
        if !seen.insert(filename) {
            continue;
        }
        let chunk = args.chunks.get(filename).ok_or_else(|| {
            ClientBuildError::Invalid("parallel import references a missing native chunk".into())
        })?;
        let native = args.chunk_table.get(chunk.originate_from).ok_or_else(|| {
            ClientBuildError::Invalid("parallel import chunk has no native graph record".into())
        })?;
        let count = native.cross_chunk_imports.len();
        let internal = chunk.imports.get(..count).ok_or_else(|| {
            ClientBuildError::Invalid("parallel import lost its native dependency partition".into())
        })?;
        pending.extend(internal.iter().map(|name| name.as_str()));
    }
    Ok(seen)
}

pub(super) fn expression(
    importer: &str,
    target_call: &str,
    dependencies: &[String],
    public_path: Option<&str>,
) -> Result<String, ClientBuildError> {
    // import() creates the intrinsic Promise: accessing its constructor avoids
    // capturing an authored local binding named Promise/globalThis/window.
    let mut result = String::from("((m)=>m.constructor.all([m");
    for dependency in dependencies {
        let path = import_path(importer, dependency, public_path)?;
        write!(
            &mut result,
            ",import({})",
            ascii_literal(&path).map_err(|error| ClientBuildError::Invalid(error.to_string()))?
        )
        .map_err(|error| ClientBuildError::Invalid(error.to_string()))?;
    }
    result.push_str("]).then(v=>v[0]))(");
    result.push_str(target_call);
    result.push(')');
    Ok(result)
}

fn import_path(
    importer: &str,
    dependency: &str,
    public_path: Option<&str>,
) -> Result<String, ClientBuildError> {
    if let Some(prefix) = public_path {
        return Ok(format!("{}/{dependency}", prefix.trim_end_matches('/')));
    }
    let parent = Path::new(importer)
        .parent()
        .ok_or_else(|| ClientBuildError::Invalid("parallel import output has no parent".into()))?;
    let target = Path::new(dependency);
    let mut from = parent.components().peekable();
    let mut to = target.components().peekable();
    while from.peek().is_some() && from.peek() == to.peek() {
        from.next();
        to.next();
    }
    let mut result = String::new();
    for _ in from {
        result.push_str("../");
    }
    if result.is_empty() {
        result.push_str("./");
    }
    for part in to {
        result.push_str(resources::utf8(Path::new(part.as_os_str()))?);
        result.push('/');
    }
    result.pop();
    Ok(result)
}
