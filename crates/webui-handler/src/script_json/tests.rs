// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use std::io::Write;

use serde::ser::{Error as _, SerializeSeq};
use serde_json::json;

use super::*;

#[derive(Default)]
struct Sink {
    output: String,
    writes: usize,
    fail_at: Option<usize>,
}

impl ResponseWriter for Sink {
    fn write(&mut self, content: &str) -> Result<()> {
        self.writes += 1;
        if self.fail_at == Some(self.writes) {
            return Err(HandlerError::ClientDisconnected);
        }
        self.output.push_str(content);
        Ok(())
    }

    fn stream_flush(&mut self) -> Result<()> {
        Err(HandlerError::Writer(
            "JSON buffering must not flush the response".to_string(),
        ))
    }

    fn end(&mut self) -> Result<()> {
        Err(HandlerError::Writer(
            "JSON buffering must not end the response".to_string(),
        ))
    }
}

#[test]
fn large_collections_bound_scratch_and_reuse_it_for_later_values() {
    let value = json!({
        "items": (0..1000).map(|index| json!({
            "index": index,
            "text": "</script>\u{1f600}&\"\n",
            "values": [null, true, false, -23, 4.5],
        })).collect::<Vec<_>>(),
    });
    let expected = serde_json::to_string(&value).unwrap().replace("</", "<\\/");
    let mut sink = Sink::default();
    let mut scratch = Vec::new();
    write_script_safe_json(&mut sink, &mut scratch, &value).unwrap();
    assert_eq!(sink.output, expected);
    assert!(scratch.capacity() <= SCRATCH_LIMIT);
    let capacity = scratch.capacity();
    sink.output.clear();
    write_script_safe_json(&mut sink, &mut scratch, &json!({"small": 1})).unwrap();
    assert_eq!(sink.output, r#"{"small":1}"#);
    assert_eq!(scratch.capacity(), capacity);
}

#[test]
fn large_string_fragments_bypass_scratch_without_changing_json() {
    let value = json!({"text": "<</script>\u{1f600}".repeat(SCRATCH_LIMIT)});
    let mut sink = Sink::default();
    let mut scratch = Vec::new();
    write_script_safe_json(&mut sink, &mut scratch, &value).unwrap();
    assert_eq!(
        sink.output,
        serde_json::to_string(&value).unwrap().replace("</", "<\\/")
    );
    assert!(scratch.capacity() <= SCRATCH_LIMIT);
}

#[test]
fn odd_capacity_does_not_double_past_the_limit() {
    let mut sink = Sink::default();
    let mut scratch = Vec::with_capacity(SCRATCH_LIMIT / 2 + 1);
    let mut writer = JsonWriter {
        writer: &mut sink,
        scratch: &mut scratch,
        error: None,
        utf8: Checked,
    };
    writer.write_all(&vec![b'x'; SCRATCH_LIMIT / 2]).unwrap();
    writer.write_all(&vec![b'y'; SCRATCH_LIMIT / 2]).unwrap();
    writer.drain(true).unwrap();
    assert!(scratch.capacity() <= SCRATCH_LIMIT);
    assert_eq!(sink.output.len(), SCRATCH_LIMIT);
}

#[test]
fn oversized_incoming_scratch_is_bounded_before_serialization() {
    let mut sink = Sink::default();
    let mut scratch = Vec::with_capacity(SCRATCH_LIMIT * 2);
    write_script_safe_json(&mut sink, &mut scratch, &"value").unwrap();
    assert_eq!(sink.output, "\"value\"");
    assert!(scratch.capacity() <= SCRATCH_LIMIT);
}

#[test]
fn closing_tags_remain_escaped_across_every_utf8_chunk_boundary() {
    let source = format!(
        "{}<</script>\u{1f600}</STYLE>{}</template><",
        "x".repeat(SCRATCH_LIMIT - 1),
        "y".repeat(SCRATCH_LIMIT + 1),
    );
    let expected = source.replace("</", "<\\/");
    for split in 0..=source.len() {
        if !source.is_char_boundary(split) {
            continue;
        }
        let mut sink = Sink::default();
        let mut scratch = Vec::new();
        let mut writer = JsonWriter {
            writer: &mut sink,
            scratch: &mut scratch,
            error: None,
            utf8: Checked,
        };
        writer.write_all(source[..split].as_bytes()).unwrap();
        writer.flush().unwrap();
        writer.write_all(source[split..].as_bytes()).unwrap();
        writer.drain(true).unwrap();
        assert_eq!(sink.output, expected, "split at {split}");
        assert!(scratch.capacity() <= SCRATCH_LIMIT);
    }
}

#[test]
fn transport_failure_keeps_its_original_variant_and_stops_writing() {
    let value = json!({"items": vec!["a"; SCRATCH_LIMIT]});
    let mut sink = Sink {
        fail_at: Some(2),
        ..Sink::default()
    };
    let mut scratch = Vec::new();
    assert!(matches!(
        write_script_safe_json(&mut sink, &mut scratch, &value),
        Err(HandlerError::ClientDisconnected)
    ));
    assert_eq!(sink.writes, 2);
    assert!(scratch.capacity() <= SCRATCH_LIMIT);
}

#[test]
fn final_drain_failure_keeps_its_original_variant() {
    let mut sink = Sink {
        fail_at: Some(1),
        ..Sink::default()
    };
    assert!(matches!(
        write_script_safe_json(&mut sink, &mut Vec::new(), &42),
        Err(HandlerError::ClientDisconnected)
    ));
    assert_eq!(sink.writes, 1);
}

struct FailingValue;

impl Serialize for FailingValue {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(None)?;
        sequence.serialize_element(&vec![true; SCRATCH_LIMIT])?;
        Err(S::Error::custom("intentional serialization failure"))
    }
}

#[test]
fn serialization_failure_is_not_reported_as_success_after_partial_output() {
    let mut sink = Sink::default();
    let mut scratch = Vec::new();
    let error = write_script_safe_json(&mut sink, &mut scratch, &FailingValue).unwrap_err();
    assert!(matches!(error, HandlerError::Rendering(_)));
    assert!(error
        .to_string()
        .contains("intentional serialization failure"));
    assert!(!sink.output.is_empty());
    assert!(scratch.capacity() <= SCRATCH_LIMIT);
}

#[derive(Clone, Copy, Debug)]
enum FailurePath {
    Buffered,
    Large,
    Flush,
}

#[test]
fn poisoned_writer_rejects_small_empty_large_writes_and_flush() {
    let full = vec![b'x'; SCRATCH_LIMIT];
    let large = vec![b'y'; SCRATCH_LIMIT + 1];
    for path in [
        FailurePath::Buffered,
        FailurePath::Large,
        FailurePath::Flush,
    ] {
        let mut sink = Sink {
            fail_at: Some(1),
            ..Sink::default()
        };
        let mut scratch = Vec::with_capacity(SCRATCH_LIMIT);
        {
            let mut writer = JsonWriter {
                writer: &mut sink,
                scratch: &mut scratch,
                error: None,
                utf8: Checked,
            };
            writer.write_all(b"").unwrap();
            let failure = match path {
                FailurePath::Buffered => {
                    writer.write_all(&full).unwrap();
                    writer.write_all(b"!")
                }
                FailurePath::Large => writer.write_all(&large),
                FailurePath::Flush => {
                    writer.write_all(b"buffered").unwrap();
                    writer.flush()
                }
            };
            assert!(failure.is_err(), "{path:?}");
            assert!(writer.scratch.is_empty(), "{path:?}");
            assert_eq!(writer.scratch.capacity(), 0, "{path:?}");
            for bytes in [b"small".as_slice(), b"".as_slice(), large.as_slice()] {
                assert!(writer.write_all(bytes).is_err(), "{path:?}");
                assert!(writer.write(bytes).is_err(), "{path:?}");
            }
            assert!(writer.flush().is_err(), "{path:?}");
            assert!(matches!(
                writer.error.as_ref(),
                Some(HandlerError::ClientDisconnected)
            ));
            assert_eq!(writer.scratch.capacity(), 0, "{path:?}");
        }
        assert_eq!(sink.writes, 1, "{path:?}");
    }
}

struct CatchingTransportFailure;

impl Serialize for CatchingTransportFailure {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(None)?;
        assert!(sequence
            .serialize_element(&vec![true; SCRATCH_LIMIT])
            .is_err());
        assert!(sequence.serialize_element("small").is_err());
        assert!(sequence.serialize_element("").is_err());
        assert!(sequence
            .serialize_element(&"x".repeat(SCRATCH_LIMIT + 1))
            .is_err());
        assert!(sequence.end().is_err());
        Err(S::Error::custom(
            "serialization continued after transport failure",
        ))
    }
}

#[test]
fn caught_transport_failure_cannot_be_masked_by_a_later_serialization_error() {
    let mut sink = Sink {
        fail_at: Some(1),
        ..Sink::default()
    };
    let mut scratch = Vec::new();
    assert!(matches!(
        write_script_safe_json(&mut sink, &mut scratch, &CatchingTransportFailure),
        Err(HandlerError::ClientDisconnected)
    ));
    assert_eq!(sink.writes, 1);
    assert_eq!(scratch.capacity(), 0);
}

#[path = "utf8_policy_tests.rs"]
mod utf8_policy;

#[path = "script_safe_search_tests.rs"]
mod script_safe_search;
