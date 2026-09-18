// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Repeat state drained within one render step, never across a host call.

use serde_json::Value;
use webui_protocol::WebUIFragmentFor;

use crate::state_view::SharedValue;
use crate::VisibleLoopScope;

pub(crate) struct RepeatFrame<'protocol, 'state> {
    pub(crate) slot: u32,
    pub(crate) declaration: &'protocol WebUIFragmentFor,
    pub(crate) items: RepeatSource<'state>,
    pub(crate) index: usize,
    pub(crate) saved_value: Option<SharedValue>,
    pub(crate) visible: VisibleLoopScope,
}

pub(crate) enum RepeatSource<'state> {
    Borrowed(&'state [Value]),
    Shared(SharedValue),
}

#[derive(Default)]
pub(crate) struct RepeatScratch {
    frames: Vec<RepeatFrame<'static, 'static>>,
}

impl RepeatScratch {
    pub(crate) fn take<'protocol, 'state>(&mut self) -> Vec<RepeatFrame<'protocol, 'state>> {
        std::mem::take(&mut self.frames)
    }

    #[expect(
        clippy::unnecessary_filter_map,
        reason = "The map changes frame lifetimes; a filter retains the borrowed input type."
    )]
    pub(crate) fn recycle(&mut self, frames: Vec<RepeatFrame<'_, '_>>) {
        // In-place collection retains only the allocation, never a borrowed frame.
        // Consuming the frames also drops any remaining captured values.
        self.frames = frames
            .into_iter()
            .filter_map(|_| None::<RepeatFrame<'static, 'static>>)
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use webui_test_utils::test_json;

    #[test]
    fn repeat_frame_borrows_protocol_names_and_state_without_collection_indirection() {
        let declaration = WebUIFragmentFor {
            item: "item".to_owned(),
            collection: "items".to_owned(),
            fragment_id: "body".to_owned(),
        };
        let items = [test_json!({"value": 1})];
        let frame = RepeatFrame {
            slot: 0,
            declaration: &declaration,
            items: RepeatSource::Borrowed(&items),
            index: 0,
            saved_value: None,
            visible: VisibleLoopScope::EMPTY,
        };
        assert!(std::mem::size_of::<RepeatFrame<'_, '_>>() <= 96);
        assert!(std::ptr::eq(frame.declaration, &declaration));
        let RepeatSource::Borrowed(borrowed) = frame.items else {
            panic!("repeat should borrow the input");
        };
        assert!(std::ptr::eq(borrowed.as_ptr(), items.as_ptr()));
    }

    #[test]
    fn repeat_scratch_reuses_empty_storage_across_input_lifetimes() {
        let mut scratch = RepeatScratch::default();
        assert_eq!(scratch.frames.capacity(), 0);
        let address;
        let capacity;
        {
            let declaration = WebUIFragmentFor {
                item: "item".to_owned(),
                collection: "items".to_owned(),
                fragment_id: "body".to_owned(),
            };
            let items = [test_json!({"value": 1})];
            let mut frames = scratch.take();
            crate::render_buffer::push(
                &mut frames,
                RepeatFrame {
                    slot: 0,
                    declaration: &declaration,
                    items: RepeatSource::Borrowed(&items),
                    index: 0,
                    saved_value: None,
                    visible: VisibleLoopScope::EMPTY,
                },
            );
            address = frames.as_ptr().addr();
            capacity = frames.capacity();
            frames.clear();
            scratch.recycle(frames);
        }
        let frames = scratch.take();
        assert!(frames.is_empty());
        assert_eq!(frames.as_ptr().addr(), address);
        assert_eq!(frames.capacity(), capacity);
        scratch.recycle(frames);
        assert_eq!(scratch.frames.as_ptr().addr(), address);
        assert_eq!(scratch.frames.capacity(), capacity);
    }

    #[test]
    fn recycling_discards_captures_and_retains_only_empty_capacity() {
        let declaration = WebUIFragmentFor {
            item: "item".to_owned(),
            collection: "items".to_owned(),
            fragment_id: "body".to_owned(),
        };
        let source = SharedValue::new(test_json!([1]));
        let previous = SharedValue::new(test_json!("outer"));
        let source_origin = std::sync::Arc::downgrade(source.origin());
        let previous_origin = std::sync::Arc::downgrade(previous.origin());
        let mut frames = Vec::new();
        crate::render_buffer::push(
            &mut frames,
            RepeatFrame {
                slot: 0,
                declaration: &declaration,
                items: RepeatSource::Shared(source),
                index: 0,
                saved_value: Some(previous),
                visible: VisibleLoopScope::EMPTY,
            },
        );
        let address = frames.as_ptr().addr();
        let capacity = frames.capacity();
        let mut scratch = RepeatScratch::default();
        scratch.recycle(frames);
        assert!(source_origin.upgrade().is_none());
        assert!(previous_origin.upgrade().is_none());
        assert!(scratch.frames.is_empty());
        assert_eq!(scratch.frames.as_ptr().addr(), address);
        assert_eq!(scratch.frames.capacity(), capacity);
    }
}
