// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Cold constructors for actionable streaming-session failures.

use crate::{HandlerError, StreamingBoundaryError};

#[cold]
#[inline(never)]
pub(crate) fn streaming_boundary_error(signal: &str, reason: &str) -> HandlerError {
    HandlerError::StreamingBoundary(Box::new(StreamingBoundaryError {
        signal: signal.to_string(),
        reason: reason.to_string(),
    }))
}

#[cold]
#[inline(never)]
pub(super) fn component_style_inventory_error(reason: &'static str) -> HandlerError {
    HandlerError::Invariant(reason.to_string())
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
    streaming_boundary_error(
        tag,
        "component host has no boundary or generated component span that can activate it; \
         place it inside <boundary> or inside a component that contains a runtime boundary",
    )
}

#[cold]
#[inline(never)]
pub(super) fn missing_streaming_root_error(tag: &str) -> HandlerError {
    streaming_boundary_error(
        tag,
        "component host has no compiler-owned streaming_root signal; rebuild the protocol with \
         matching parser and handler versions",
    )
}

#[cold]
#[inline(never)]
pub(super) fn misplaced_streaming_root_error(tag: &str, expected: &str) -> HandlerError {
    streaming_boundary_error(
        tag,
        &format!("component root signal is misplaced; expected {expected}"),
    )
}

#[cold]
#[inline(never)]
pub(super) fn mismatched_streaming_root_error(
    signal_tag: &str,
    component_tag: &str,
) -> HandlerError {
    streaming_boundary_error(
        signal_tag,
        &format!(
            "root signal names <{signal_tag}>, but the component fragment renders \
             <{component_tag}>; rebuild the protocol"
        ),
    )
}

#[cold]
#[inline(never)]
pub(super) fn generated_streaming_root_error(tag: &str) -> HandlerError {
    streaming_boundary_error(
        tag,
        "handler-generated route host was not prepared for streaming",
    )
}

#[cold]
#[inline(never)]
pub(super) fn boundary_not_updatable_error(boundary_id: usize) -> HandlerError {
    streaming_boundary_error(
        "update",
        &format!(
            "boundary instance {boundary_id} was committed as final; resume it with \
             BoundaryMode::Updatable before sending updates"
        ),
    )
}

#[cold]
#[inline(never)]
pub(super) fn state_update_type_error() -> HandlerError {
    HandlerError::TypeError("streaming state updates require a JSON object patch".to_string())
}

#[cold]
#[inline(never)]
pub(super) fn boundary_order_error(operation: &str, reason: &str) -> HandlerError {
    streaming_boundary_error(operation, reason)
}

#[cold]
#[inline(never)]
pub(super) fn continuation_limit_error(limit: usize) -> HandlerError {
    streaming_boundary_error(
        "continuation",
        &format!(
            "continuation depth exceeds {limit}; split deeply nested components or directives"
        ),
    )
}

#[cold]
#[inline(never)]
pub(super) fn boundary_limit_error(limit: usize) -> HandlerError {
    streaming_boundary_error(
        "boundary",
        &format!("runtime boundary occurrence count exceeds {limit}; reduce repeated boundaries"),
    )
}

#[cold]
#[inline(never)]
pub(super) fn span_id_overflow_error() -> HandlerError {
    streaming_boundary_error(
        "component span",
        "component span IDs exhausted the response-local 32-bit range; split the page into \
         several responses or reduce boundary-bearing component hosts",
    )
}

#[cold]
#[inline(never)]
pub(super) fn span_nesting_error(limit: usize) -> HandlerError {
    streaming_boundary_error(
        "component span",
        &format!("component span nesting exceeds {limit}; flatten boundary-bearing components"),
    )
}

#[cold]
#[inline(never)]
pub(super) fn updatable_limit_error(limit: usize) -> HandlerError {
    streaming_boundary_error(
        "resume",
        &format!(
            "this response already committed {limit} updatable boundary occurrences, the most the \
             browser retains; resume this occurrence with BoundaryMode::Final, or split the page \
             so fewer occurrences need later updates"
        ),
    )
}

#[cold]
#[inline(never)]
pub(super) fn invalid_boundary_key_error(
    declaration_id: u32,
    name: &str,
    reason: &str,
) -> HandlerError {
    streaming_boundary_error(
        name,
        &format!(
            "boundary declaration {declaration_id} has an invalid runtime key: {reason}; \
             keys must resolve to a finite JSON number or string"
        ),
    )
}

#[cold]
#[inline(never)]
pub(super) fn duplicate_boundary_key_error(
    declaration_id: u32,
    name: &str,
    key: &str,
) -> HandlerError {
    streaming_boundary_error(
        name,
        &format!(
            "boundary declaration {declaration_id} produced duplicate key {key}; \
             every repeated occurrence must have a unique key"
        ),
    )
}

#[cold]
#[inline(never)]
pub(super) fn boundary_in_repeat_error(name: &str) -> HandlerError {
    streaming_boundary_error(
        name,
        "boundary declarations are not valid inside a <for> repeat body; rebuild the protocol \
         with a parser that rejects boundary-in-repeat, then move the boundary outside the \
         repeat or wrap the whole repeat in one boundary",
    )
}

#[cold]
#[inline(never)]
pub(super) fn keyed_instance_limit_error(limit: usize) -> HandlerError {
    streaming_boundary_error(
        "boundary key",
        &format!("keyed boundary occurrence count exceeds {limit}; reduce repeated instances"),
    )
}

#[cold]
#[inline(never)]
pub(super) fn malformed_span_signal_error(signal: &str, reason: &str) -> HandlerError {
    streaming_boundary_error(signal, reason)
}
