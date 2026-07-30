// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Immutable segment indexes for host-driven streaming responses.

use std::ops::Range;

use webui_protocol::{web_ui_fragment::Fragment, StreamingBoundaryList, WebUIFragment};

use super::error::{parse_boundary_sequence, streaming_boundary_error};
use super::state::{BOUNDARY_END_PREFIX, BOUNDARY_START_PREFIX};
use crate::{structural_signal_value, HandlerError, Result};

pub(crate) struct StreamingEntryPlan {
    shell_end: usize,
    boundaries: Vec<Range<usize>>,
}

impl StreamingEntryPlan {
    pub(crate) fn new(
        _entry_id: &str,
        fragments: &[WebUIFragment],
        names: Option<&StreamingBoundaryList>,
    ) -> Self {
        match Self::try_new(fragments, names) {
            Ok(plan) => plan,
            Err(_) => Self {
                shell_end: fragments.len(),
                boundaries: Vec::new(),
            },
        }
    }

    fn try_new(fragments: &[WebUIFragment], names: Option<&StreamingBoundaryList>) -> Result<Self> {
        let mut boundaries = Vec::new();
        let mut active = None;
        let mut next_id = 0usize;

        for (index, fragment) in fragments.iter().enumerate() {
            let Some(Fragment::Signal(signal)) = fragment.fragment.as_ref() else {
                continue;
            };
            let Some(value) = structural_signal_value(signal) else {
                continue;
            };
            if let Some(raw_id) = value.strip_prefix(BOUNDARY_START_PREFIX) {
                let id = parse_boundary_sequence(value, raw_id)?;
                if active.is_some() {
                    return Err(streaming_boundary_error(
                        value,
                        "nested boundaries are not valid in a streaming entry plan",
                    ));
                }
                if id != next_id {
                    return Err(streaming_boundary_error(
                        value,
                        "boundary IDs must be contiguous and declaration ordered",
                    ));
                }
                active = Some((id, index));
                continue;
            }
            if let Some(raw_id) = value.strip_prefix(BOUNDARY_END_PREFIX) {
                let id = parse_boundary_sequence(value, raw_id)?;
                let Some((active_id, start)) = active.take() else {
                    return Err(streaming_boundary_error(
                        value,
                        "boundary end has no matching start",
                    ));
                };
                if id != active_id {
                    return Err(streaming_boundary_error(
                        value,
                        "boundary end does not match the active boundary",
                    ));
                }
                boundaries.push(start..index + 1);
                next_id = next_id.checked_add(1).ok_or_else(|| {
                    streaming_boundary_error(value, "boundary count exceeds the platform limit")
                })?;
            }
        }

        if let Some((id, _)) = active {
            return Err(streaming_boundary_error(
                &format!("boundary_start:{id}"),
                "entry fragment ended before the matching boundary end",
            ));
        }
        if names.is_some_and(|list| list.names.len() != boundaries.len()) {
            return Err(HandlerError::Invariant(
                "streaming boundary name table does not match entry boundary count".to_string(),
            ));
        }

        Ok(Self {
            shell_end: boundaries
                .first()
                .map_or(fragments.len(), |boundary| boundary.start),
            boundaries,
        })
    }

    pub(crate) fn shell_end(&self) -> usize {
        self.shell_end
    }

    pub(crate) fn boundary(&self, id: usize) -> Option<&Range<usize>> {
        self.boundaries.get(id)
    }

    pub(crate) fn boundary_count(&self) -> usize {
        self.boundaries.len()
    }
}
