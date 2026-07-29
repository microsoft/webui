// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Cold constructors for streaming boundary diagnostics.
//!
//! These are the only paths that allocate error strings, so each is marked
//! `#[cold]` and `#[inline(never)]`: an inlined error builder perturbs the
//! layout of the hot render loop it was inlined into. See
//! `.github/skills/diagnostics/SKILL.md`.

use crate::{HandlerError, Result, StreamingBoundaryError};

use super::root::STREAMING_ROOT_PREFIX;

pub(super) fn parse_boundary_sequence(signal: &str, sequence: &str) -> Result<usize> {
    if sequence.is_empty() {
        return Err(streaming_boundary_error(signal, "missing decimal sequence"));
    }
    if !sequence.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(streaming_boundary_error(
            signal,
            "sequence must contain only decimal digits",
        ));
    }
    sequence
        .parse::<usize>()
        .map_err(|_| streaming_boundary_error(signal, "sequence exceeds the platform limit"))
}

#[cold]
#[inline(never)]
pub(crate) fn streaming_boundary_error(signal: &str, reason: &str) -> HandlerError {
    HandlerError::StreamingBoundary(Box::new(StreamingBoundaryError {
        signal: signal.to_string(),
        reason: reason.to_string(),
    }))
}

/// A streamed component host (`streaming_root:<tag>`) was authored outside any
/// `<boundary>`. Such a host can never be progressively activated, so the
/// streaming render fails with an actionable structured error instead of
/// emitting dead markup. Cold and out-of-line: reuses the boxed
/// [`HandlerError::StreamingBoundary`] variant so no error widens the small
/// hot-path `Result`.
#[cold]
#[inline(never)]
pub(super) fn streaming_root_outside_boundary_error(tag: &str) -> HandlerError {
    HandlerError::StreamingBoundary(Box::new(StreamingBoundaryError {
        signal: format!("{STREAMING_ROOT_PREFIX}{tag}"),
        reason: format!(
            "streamed component host <{tag}> is outside any <boundary>; \
             author the host, or its matched <route>, inside a boundary so it can be \
             progressively hydrated"
        ),
    }))
}

#[cold]
#[inline(never)]
pub(super) fn missing_streaming_root_error(tag: &str) -> HandlerError {
    HandlerError::StreamingBoundary(Box::new(StreamingBoundaryError {
        signal: format!("{STREAMING_ROOT_PREFIX}{tag}"),
        reason: format!(
            "component <{tag}> has no matching compiler-owned root signal at its opening-tag \
             close; rebuild the protocol and place the host inside an explicit <boundary>"
        ),
    }))
}

#[cold]
#[inline(never)]
pub(super) fn misplaced_streaming_root_error(tag: &str, expected: &str) -> HandlerError {
    HandlerError::StreamingBoundary(Box::new(StreamingBoundaryError {
        signal: format!("{STREAMING_ROOT_PREFIX}{tag}"),
        reason: format!(
            "root signal for <{tag}> is misplaced: expected {expected}; rebuild the protocol \
             with matching parser and handler versions"
        ),
    }))
}

#[cold]
#[inline(never)]
pub(super) fn mismatched_streaming_root_error(
    signal_tag: &str,
    component_tag: &str,
) -> HandlerError {
    HandlerError::StreamingBoundary(Box::new(StreamingBoundaryError {
        signal: format!("{STREAMING_ROOT_PREFIX}{signal_tag}"),
        reason: format!(
            "root signal names <{signal_tag}>, but the opening host renders component \
             <{component_tag}>; rebuild the protocol so the tags match"
        ),
    }))
}

#[cold]
#[inline(never)]
pub(super) fn generated_streaming_root_error(tag: &str) -> HandlerError {
    HandlerError::StreamingBoundary(Box::new(StreamingBoundaryError {
        signal: format!("{STREAMING_ROOT_PREFIX}{tag}"),
        reason: format!(
            "handler-generated route host <{tag}> was not marked for streaming before it \
             rendered; place the matched <route> inside an explicit <boundary>"
        ),
    }))
}
