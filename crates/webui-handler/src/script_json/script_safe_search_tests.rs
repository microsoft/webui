// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::Sink;
use crate::{write_script_safe_json_str, HandlerError, ResponseWriter, Result};

struct ExactWrites<'a> {
    sink: Sink,
    chunks: &'a [&'a str],
}

impl ResponseWriter for ExactWrites<'_> {
    fn write(&mut self, content: &str) -> Result<()> {
        assert_eq!(self.chunks.get(self.sink.writes).copied(), Some(content));
        self.sink.write(content)
    }

    fn stream_flush(&mut self) -> Result<()> {
        self.sink.stream_flush()
    }

    fn end(&mut self) -> Result<()> {
        self.sink.end()
    }
}

#[test]
fn script_safe_search_keeps_exact_write_segments() -> Result<()> {
    let cases: &[(&str, &[&str])] = &[
        ("", &[]),
        ("a\u{1f600}b", &["a\u{1f600}b"]),
        ("<", &["<"]),
        ("<<<x/", &["<<<x/"]),
        ("</", &["<\\/"]),
        ("</a>", &["<\\/", "a>"]),
        ("<</", &["<", "<\\/"]),
        ("</</", &["<\\/", "<\\/"]),
        ("<//<", &["<\\/", "/<"]),
        (
            "abc</script>def</style>",
            &["abc", "<\\/", "script>def", "<\\/", "style>"],
        ),
        (
            "\u{1f600}<\u{00e9}</SCRIPT>",
            &["\u{1f600}<\u{00e9}", "<\\/", "SCRIPT>"],
        ),
        ("<\u{1f600}/", &["<\u{1f600}/"]),
    ];
    for &(source, chunks) in cases {
        let mut writer = ExactWrites {
            sink: Sink::default(),
            chunks,
        };
        write_script_safe_json_str(&mut writer, source)?;
        assert_eq!(writer.sink.writes, chunks.len());
        assert_eq!(writer.sink.output, source.replace("</", "<\\/"));
    }
    Ok(())
}

#[test]
fn script_safe_search_handles_long_nonmatching_delimiters() -> Result<()> {
    for pattern in ["<", "/", "<x/", "/<x", "\u{1f600}<x"] {
        let source = pattern.repeat(4096);
        let chunks = [source.as_str()];
        let mut writer = ExactWrites {
            sink: Sink::default(),
            chunks: &chunks,
        };
        write_script_safe_json_str(&mut writer, &source)?;
        assert_eq!(writer.sink.writes, 1);
        assert_eq!(writer.sink.output, source);
    }
    Ok(())
}

#[test]
fn script_safe_search_preserves_transport_errors_and_stops() {
    let source = "<i>x</i> <n>y</n>";
    let chunks = ["<i>x", "<\\/", "i> <n>y", "<\\/", "n>"];
    for fail_at in 1..=chunks.len() {
        let mut writer = ExactWrites {
            sink: Sink {
                fail_at: Some(fail_at),
                ..Sink::default()
            },
            chunks: &chunks,
        };
        assert!(matches!(
            write_script_safe_json_str(&mut writer, source),
            Err(HandlerError::ClientDisconnected)
        ));
        assert_eq!(writer.sink.writes, fail_at);
        assert_eq!(writer.sink.output, chunks[..fail_at - 1].concat());
    }
}

#[test]
fn script_safe_search_matches_reference_for_short_ascii_inputs() -> Result<()> {
    let alphabet = ['<', '/', 'x'];
    let mut combinations = 1;
    for length in 0..=7 {
        for mut ordinal in 0..combinations {
            let mut source = String::with_capacity(length);
            for _ in 0..length {
                source.push(alphabet[ordinal % alphabet.len()]);
                ordinal /= alphabet.len();
            }
            let mut sink = Sink::default();
            write_script_safe_json_str(&mut sink, &source)?;
            assert_eq!(sink.output, source.replace("</", "<\\/"), "{source}");
        }
        combinations *= alphabet.len();
    }
    Ok(())
}
