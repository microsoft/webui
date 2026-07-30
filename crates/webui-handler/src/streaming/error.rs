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

#[cold]
#[inline(never)]
pub(super) fn boundary_not_updatable_error(boundary_id: usize) -> HandlerError {
    HandlerError::StreamingBoundary(Box::new(StreamingBoundaryError {
        signal: format!("state_update:{boundary_id}"),
        reason: "the target boundary was committed as final; commit it with \
                 BoundaryMode::Updatable before sending state updates"
            .to_string(),
    }))
}

#[cold]
#[inline(never)]
pub(super) fn state_update_type_error() -> HandlerError {
    HandlerError::TypeError(
        "streaming state updates require a JSON object so projected keys can be \
         applied through component setState()"
            .to_string(),
    )
}

#[cold]
#[inline(never)]
pub(super) fn boundary_order_error(operation: &str, reason: &str) -> HandlerError {
    HandlerError::StreamingBoundary(Box::new(StreamingBoundaryError {
        signal: operation.to_string(),
        reason: reason.to_string(),
    }))
}

#[cold]
#[inline(never)]
pub(super) fn unknown_boundary_name_error(name: &str, valid: &[String]) -> HandlerError {
    let suggestion = closest_name(name, valid);
    let mut reason = format!("unknown boundary name `{name}`");
    if let Some(suggestion) = suggestion {
        reason.push_str("; did you mean `");
        reason.push_str(suggestion);
        reason.push_str("`?");
    }
    if !valid.is_empty() {
        reason.push_str(" valid names: ");
        for (index, candidate) in valid.iter().enumerate() {
            if index != 0 {
                reason.push_str(", ");
            }
            reason.push('`');
            reason.push_str(candidate);
            reason.push('`');
        }
    }
    HandlerError::StreamingBoundary(Box::new(StreamingBoundaryError {
        signal: name.to_string(),
        reason,
    }))
}

fn closest_name<'a>(target: &str, candidates: &'a [String]) -> Option<&'a str> {
    candidates
        .iter()
        .min_by_key(|candidate| levenshtein(target, candidate))
        .map(String::as_str)
}

fn levenshtein(left: &str, right: &str) -> usize {
    let right_len = right.chars().count();
    let mut previous: Vec<usize> = (0..=right_len).collect();
    let mut current = vec![0; right_len + 1];
    for (left_index, left_char) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_char) in right.chars().enumerate() {
            current[right_index + 1] = if left_char == right_char {
                previous[right_index]
            } else {
                1 + previous[right_index]
                    .min(previous[right_index + 1])
                    .min(current[right_index])
            };
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right_len]
}
