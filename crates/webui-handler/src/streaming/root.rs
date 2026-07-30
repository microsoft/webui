// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Streamed SSR host marking (` data-ws`).
//!
//! The parser emits a compiler-owned `streaming_root:` signal inside a
//! component's opening tag, immediately before `>`. Streaming rendering
//! consumes it to inject ` data-ws` so the deferral marker exists in the byte
//! stream *before* the browser can upgrade the custom element. Ordinary
//! rendering ignores the signal byte-for-byte.

use webui_protocol::{web_ui_fragment::Fragment, WebUIFragment};

use super::error::{
    generated_streaming_root_error, mismatched_streaming_root_error,
    misplaced_streaming_root_error, missing_streaming_root_error, streaming_boundary_error,
    streaming_root_outside_boundary_error,
};
use super::{require_streaming_head_start, streaming_state};
use crate::{structural_signal_value, Result, WebUIProcessContext};

/// Compiler-owned signal marking a streamed SSR component host. The parser
/// emits it inside the component's opening tag, immediately before `>`. Ordinary
/// rendering ignores it byte-for-byte; streaming rendering consumes it to inject
/// ` data-ws` so the marker exists before custom-element upgrade.
pub(super) const STREAMING_ROOT_PREFIX: &str = "streaming_root:";

#[derive(Clone, Copy)]
pub(super) enum StreamingRootStage {
    OpeningTagClose,
    Component,
}

/// Parser-produced root signal awaiting its exact `>`/component sequence.
#[derive(Clone, Copy)]
pub(super) struct PendingStreamingRoot<'data> {
    pub(super) tag: &'data str,
    pub(super) stage: StreamingRootStage,
}

/// Which side produced a component host: the parser (from authored markup) or
/// the handler itself (a generated route host). Only parser-produced hosts
/// carry a `streaming_root:` signal.
#[derive(Clone, Copy)]
pub(crate) enum ComponentHostOrigin {
    ParserProduced,
    HandlerGenerated,
}

#[inline]
pub(crate) fn validate_pending_streaming_root(
    fragment: &WebUIFragment,
    context: &mut WebUIProcessContext<'_, '_, '_>,
) -> Result<()> {
    let Some(pending) = context
        .streaming
        .as_ref()
        .and_then(|streaming| streaming.pending_root)
    else {
        return Ok(());
    };

    match (pending.stage, fragment.fragment.as_ref()) {
        (StreamingRootStage::OpeningTagClose, Some(Fragment::Raw(raw)))
            if raw.value == ">" || raw.value == "/>" =>
        {
            if let Some(root) = context
                .streaming
                .as_mut()
                .and_then(|streaming| streaming.pending_root.as_mut())
            {
                root.stage = StreamingRootStage::Component;
            }
            Ok(())
        }
        (StreamingRootStage::Component, Some(Fragment::Component(_))) => Ok(()),
        (StreamingRootStage::OpeningTagClose, _) => Err(misplaced_streaming_root_error(
            pending.tag,
            "the raw opening-tag close `>` or `/>` immediately after the signal",
        )),
        (StreamingRootStage::Component, _) => Err(misplaced_streaming_root_error(
            pending.tag,
            "the matching component fragment immediately after the opening-tag close",
        )),
    }
}

fn raw_has_unclosed_opening_tag(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    let mut search_end = raw.len();
    while let Some(open) = raw[..search_end].rfind('<') {
        let name_start = open + 1;
        if bytes.get(name_start).is_some_and(|byte| {
            !byte.is_ascii_whitespace() && !matches!(*byte, b'/' | b'!' | b'?' | b'>')
        }) {
            let mut name_end = name_start;
            while name_end < bytes.len()
                && !bytes[name_end].is_ascii_whitespace()
                && !matches!(bytes[name_end], b'/' | b'>')
            {
                name_end += 1;
            }
            if !contains_unquoted_tag_close(&bytes[name_end..]) {
                return true;
            }
        }
        search_end = open;
    }
    false
}

fn contains_unquoted_tag_close(bytes: &[u8]) -> bool {
    let mut quote = 0u8;
    for &byte in bytes {
        if quote != 0 {
            if byte == quote {
                quote = 0;
            }
        } else if matches!(byte, b'"' | b'\'') {
            quote = byte;
        } else if byte == b'>' {
            return true;
        }
    }
    false
}

#[inline]
pub(crate) fn validate_streaming_root_opening(
    preceding: &[WebUIFragment],
    fragment: &WebUIFragment,
) -> Result<()> {
    let Some(Fragment::Signal(signal)) = fragment.fragment.as_ref() else {
        return Ok(());
    };
    let Some(tag) =
        structural_signal_value(signal).and_then(|value| value.strip_prefix(STREAMING_ROOT_PREFIX))
    else {
        return Ok(());
    };
    if tag.is_empty() {
        return Ok(());
    }

    for candidate in preceding.iter().rev() {
        match candidate.fragment.as_ref() {
            Some(Fragment::Attribute(_) | Fragment::Plugin(_)) => {}
            Some(Fragment::Raw(raw)) if raw_has_unclosed_opening_tag(&raw.value) => return Ok(()),
            Some(Fragment::Raw(raw))
                if raw
                    .value
                    .as_bytes()
                    .first()
                    .is_some_and(|byte| byte.is_ascii_whitespace()) => {}
            _ => break,
        }
    }
    Err(misplaced_streaming_root_error(
        tag,
        "the compiler-owned signal at an unclosed component opening-tag close",
    ))
}

/// Assert that no component root is mid-flight.
///
/// Runs unconditionally at the end of every fragment list, so the non-streaming
/// and clean-streaming fast paths stay inlinable and the two failure shapes are
/// built out-of-line.
#[inline]
pub(crate) fn ensure_no_pending_streaming_root(
    context: &WebUIProcessContext<'_, '_, '_>,
    before: &str,
) -> Result<()> {
    let Some(streaming) = context.streaming.as_ref() else {
        return Ok(());
    };
    if streaming.pending_root.is_none() && !streaming.generated_root_ready {
        return Ok(());
    }
    unfinished_streaming_root_error(streaming.pending_root.map(|pending| pending.tag), before)
}

#[cold]
#[inline(never)]
fn unfinished_streaming_root_error(pending_tag: Option<&str>, before: &str) -> Result<()> {
    match pending_tag {
        Some(tag) => Err(misplaced_streaming_root_error(tag, before)),
        None => Err(streaming_boundary_error(
            "handler-generated route root",
            "route host ended before its component fragment rendered",
        )),
    }
}

pub(crate) fn prepare_generated_streaming_root(
    tag: &str,
    context: &mut WebUIProcessContext<'_, '_, '_>,
) -> Result<()> {
    if context.streaming.is_none() {
        return Ok(());
    }
    require_streaming_head_start(context, "route component host")?;
    let (active_boundary, generated_root_ready) = context
        .streaming
        .as_ref()
        .map_or((None, false), |streaming| {
            (streaming.active_boundary, streaming.generated_root_ready)
        });
    if active_boundary.is_none() {
        return Err(streaming_root_outside_boundary_error(tag));
    }
    if generated_root_ready {
        return Err(generated_streaming_root_error(tag));
    }
    context.writer.write(" data-ws")?;
    streaming_state(context)?.generated_root_ready = true;
    Ok(())
}

pub(crate) fn consume_streaming_component_root(
    tag: &str,
    origin: ComponentHostOrigin,
    context: &mut WebUIProcessContext<'_, '_, '_>,
) -> Result<()> {
    let streaming = streaming_state(context)?;
    match origin {
        ComponentHostOrigin::ParserProduced => {
            let Some(pending) = streaming.pending_root.take() else {
                return Err(missing_streaming_root_error(tag));
            };
            if !matches!(pending.stage, StreamingRootStage::Component) {
                return Err(misplaced_streaming_root_error(
                    pending.tag,
                    "the opening-tag close before the component fragment",
                ));
            }
            if pending.tag != tag {
                return Err(mismatched_streaming_root_error(pending.tag, tag));
            }
        }
        ComponentHostOrigin::HandlerGenerated => {
            if !streaming.generated_root_ready {
                return Err(generated_streaming_root_error(tag));
            }
            streaming.generated_root_ready = false;
        }
    }
    Ok(())
}

pub(super) fn process_streaming_root_signal<'data>(
    value: &'data str,
    context: &mut WebUIProcessContext<'data, '_, '_>,
) -> Result<()> {
    require_streaming_head_start(context, "component host")?;
    let Some(tag) = value.strip_prefix(STREAMING_ROOT_PREFIX) else {
        return Err(streaming_boundary_error(
            value,
            "expected `streaming_root:<component tag>`",
        ));
    };
    if tag.is_empty() {
        return Err(streaming_boundary_error(
            value,
            "expected `streaming_root:<component tag>`",
        ));
    }

    let (active_boundary, pending_root, generated_root_ready) =
        context
            .streaming
            .as_ref()
            .map_or((None, None, false), |streaming| {
                (
                    streaming.active_boundary,
                    streaming.pending_root,
                    streaming.generated_root_ready,
                )
            });
    if active_boundary.is_none() {
        return Err(streaming_root_outside_boundary_error(tag));
    }
    if let Some(pending) = pending_root {
        return Err(misplaced_streaming_root_error(
            pending.tag,
            "one opening-tag close and matching component before another root signal",
        ));
    }
    if generated_root_ready {
        return Err(generated_streaming_root_error(tag));
    }

    context.writer.write(" data-ws")?;
    streaming_state(context)?.pending_root = Some(PendingStreamingRoot {
        tag,
        stage: StreamingRootStage::OpeningTagClose,
    });
    Ok(())
}
