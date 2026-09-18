// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;

struct Chunks<'a>(&'a [&'a str]);

impl std::fmt::Display for Chunks<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for chunk in self.0 {
            formatter.write_str(chunk)?;
        }
        Ok(())
    }
}

impl Serialize for Chunks<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[test]
fn collect_str_preserves_unicode_and_tags_across_buffer_and_large_writes() {
    let large = format!("/script>{}<", "\u{1f600}".repeat(SCRATCH_LIMIT));
    for length in SCRATCH_LIMIT - 5..=SCRATCH_LIMIT + 1 {
        let padding = "x".repeat(length);
        for suffix in ["/script>\u{1f600}<", large.as_str()] {
            // At limit - 4, quote + padding + e-acute + '<' exactly fills scratch.
            let chunks = [
                padding.as_str(),
                "\u{e9}",
                "<",
                suffix,
                "/STYLE>\n\\\"\u{0}",
            ];
            let value = Chunks(&chunks);
            let mut sink = Sink::default();
            let mut scratch = Vec::with_capacity(SCRATCH_LIMIT);
            write_script_safe_json(&mut sink, &mut scratch, &value).unwrap();
            assert_eq!(
                sink.output,
                serde_json::to_string(&value).unwrap().replace("</", "<\\/"),
                "padding {length}, suffix bytes {}",
                suffix.len(),
            );
            assert!(scratch.capacity() <= SCRATCH_LIMIT);
        }
    }
}

#[test]
fn raw_value_preserves_large_unicode_fragments_and_clears_dirty_scratch() {
    let raw = serde_json::value::RawValue::from_string(format!(
        r#"{{"text": "{}", "bytes": [0, 255]}}"#,
        "\u{e9}</script>\u{1f600}".repeat(SCRATCH_LIMIT),
    ))
    .unwrap();
    let mut sink = Sink::default();
    let mut scratch = vec![0xff; SCRATCH_LIMIT + 1];
    write_script_safe_json(&mut sink, &mut scratch, &raw).unwrap();
    assert_eq!(sink.output, raw.get().replace("</", "<\\/"));
    assert!(scratch.is_empty());
    assert!(scratch.capacity() <= SCRATCH_LIMIT);
}

struct Bytes<'a>(&'a [u8]);

impl Serialize for Bytes<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_bytes(self.0)
    }
}

#[test]
fn non_utf8_byte_values_serialize_as_json_numbers_across_drains() {
    let bytes = [0, 127, 128, 255, b'<', b'/'].repeat(SCRATCH_LIMIT / 6 + 1);
    let value = Bytes(&bytes);
    let mut sink = Sink::default();
    let mut scratch = Vec::new();
    write_script_safe_json(&mut sink, &mut scratch, &value).unwrap();
    assert_eq!(sink.output, serde_json::to_string(&value).unwrap());
    assert_eq!(
        serde_json::from_str::<Vec<u8>>(&sink.output).unwrap(),
        bytes
    );
    assert!(scratch.capacity() <= SCRATCH_LIMIT);
}

struct FailingAfterChunks<'a>(Chunks<'a>);

impl Serialize for FailingAfterChunks<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(None)?;
        sequence.serialize_element(&self.0)?;
        Err(S::Error::custom("failure after Unicode collect_str"))
    }
}

#[test]
fn unicode_collect_str_preserves_serialization_and_transport_errors() {
    let large = "\u{1f600}".repeat(SCRATCH_LIMIT);
    let chunks = [large.as_str(), "<", "/script>"];
    let value = FailingAfterChunks(Chunks(&chunks));
    for fail_at in [None, Some(1), Some(2)] {
        let mut sink = Sink {
            fail_at,
            ..Sink::default()
        };
        let mut scratch = Vec::new();
        let error = write_script_safe_json(&mut sink, &mut scratch, &value).unwrap_err();
        if let Some(writes) = fail_at {
            assert!(matches!(error, HandlerError::ClientDisconnected));
            assert_eq!(sink.writes, writes);
            assert_eq!(scratch.capacity(), 0);
        } else {
            assert!(matches!(error, HandlerError::Rendering(_)));
            assert!(error
                .to_string()
                .contains("failure after Unicode collect_str"));
            assert!(!sink.output.is_empty());
            assert!(scratch.capacity() <= SCRATCH_LIMIT);
        }
    }
}

fn checked_writer<'a>(sink: &'a mut Sink, scratch: &'a mut Vec<u8>) -> JsonWriter<'a> {
    JsonWriter {
        writer: sink,
        scratch,
        error: None,
        utf8: Checked,
    }
}

#[test]
fn zero_sized_policy_preserves_the_original_writer_layout() {
    struct PreviousWriter<'a> {
        writer: &'a mut dyn ResponseWriter,
        scratch: &'a mut Vec<u8>,
        error: Option<HandlerError>,
    }

    let mut sink = Sink::default();
    let mut scratch = Vec::new();
    let previous = PreviousWriter {
        writer: &mut sink,
        scratch: &mut scratch,
        error: None,
    };
    let size = std::mem::size_of_val(&previous);
    let alignment = std::mem::align_of_val(&previous);
    let PreviousWriter {
        writer,
        scratch,
        error,
    } = previous;
    let current = JsonWriter {
        writer,
        scratch,
        error,
        utf8: Checked,
    };
    assert_eq!(std::mem::size_of::<Checked>(), 0);
    assert_eq!(std::mem::align_of::<Checked>(), 1);
    assert_eq!(std::mem::size_of_val(&current), size);
    assert_eq!(std::mem::align_of_val(&current), alignment);
}

#[test]
fn checked_sink_rejects_invalid_buffered_bytes_only_when_drained() {
    let mut sink = Sink::default();
    let mut scratch = Vec::with_capacity(SCRATCH_LIMIT);
    {
        let mut writer = checked_writer(&mut sink, &mut scratch);
        assert_eq!(writer.write(&[0xff]).unwrap(), 1);
        let error = writer.drain(true).unwrap_err();
        assert!(matches!(
            error,
            HandlerError::Rendering(message) if message.starts_with("invalid JSON UTF-8: ")
        ));
        assert_eq!(writer.scratch.as_slice(), &[0xff]);
        assert!(writer.error.is_none());
    }
    assert_eq!(sink.writes, 0);
}

#[test]
fn checked_sink_poisoning_preserves_incomplete_utf8_flush_errors() {
    let mut sink = Sink::default();
    let mut scratch = Vec::with_capacity(SCRATCH_LIMIT);
    {
        let mut writer = checked_writer(&mut sink, &mut scratch);
        writer.write_all(&[0xe2, 0x82]).unwrap();
        assert!(writer.flush().is_err());
        let error = writer.error.as_ref().unwrap();
        assert!(matches!(
            error,
            HandlerError::Rendering(message) if message.starts_with("invalid JSON UTF-8: ")
        ));
        assert!(writer.scratch.is_empty());
        assert_eq!(writer.scratch.capacity(), 0);
        assert!(writer.write_all(&[0xac]).is_err());
        assert!(writer.write_all(b"").is_err());
        assert!(writer.write(b"").is_err());
        assert!(writer.flush().is_err());
    }
    assert_eq!(sink.writes, 0);
}

#[test]
fn checked_large_invalid_input_preserves_drain_then_decode_error_order() {
    let large = vec![0xff; SCRATCH_LIMIT + 1];
    for prefix in ["", "<", "pending"] {
        let mut sink = Sink {
            fail_at: Some(1),
            ..Sink::default()
        };
        let mut scratch = Vec::with_capacity(SCRATCH_LIMIT);
        {
            let mut writer = checked_writer(&mut sink, &mut scratch);
            writer.write_all(prefix.as_bytes()).unwrap();
            assert!(writer.write_all(&large).is_err());
            let error = writer.error.as_ref().unwrap();
            if prefix == "pending" {
                assert!(matches!(error, HandlerError::ClientDisconnected));
            } else {
                assert!(matches!(
                    error,
                    HandlerError::Rendering(message) if message.starts_with("invalid JSON UTF-8: ")
                ));
            }
            assert!(writer.scratch.is_empty());
            assert_eq!(writer.scratch.capacity(), 0);
            assert!(writer.write_all(b"").is_err());
        }
        assert_eq!(sink.writes, usize::from(prefix == "pending"));
        assert!(sink.output.is_empty());
    }
}

#[test]
fn checked_sink_accepts_split_code_points_completed_before_a_drain() {
    let source = "\u{e9}\u{1f600}</script>";
    let expected = source.replace("</", "<\\/");
    for split in 0..=source.len() {
        let mut sink = Sink::default();
        let mut scratch = Vec::with_capacity(SCRATCH_LIMIT);
        {
            let mut writer = checked_writer(&mut sink, &mut scratch);
            writer.write_all(&source.as_bytes()[..split]).unwrap();
            writer.write_all(&source.as_bytes()[split..]).unwrap();
            writer.drain(true).unwrap();
        }
        assert_eq!(sink.output, expected, "byte split at {split}");
        assert!(scratch.capacity() <= SCRATCH_LIMIT);
    }
}
