// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use webui_handler::{FlushWriter, ResponseWriter};
use webui_parser::HtmlParser;
use webui_protocol::WebUIProtocol;

pub(crate) const ENTRY: &str = "index.html";

pub(crate) fn compile_document(source: &str) -> WebUIProtocol {
    let mut parser = HtmlParser::new();
    parser
        .parse(ENTRY, source)
        .unwrap_or_else(|error| panic!("compiling fragment benchmark failed: {error}"));
    WebUIProtocol::new(parser.into_fragment_records())
}

pub(crate) struct EvidenceWriter {
    pub(crate) output: String,
    writes: usize,
    growths: usize,
    flushes: usize,
}

impl EvidenceWriter {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            output: String::with_capacity(capacity),
            writes: 0,
            growths: 0,
            flushes: 0,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.output.clear();
        self.writes = 0;
        self.growths = 0;
        self.flushes = 0;
    }

    pub(crate) fn evidence(&self) -> (usize, usize, usize, usize) {
        (self.output.len(), self.writes, self.growths, self.flushes)
    }
}

impl ResponseWriter for EvidenceWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        let capacity = self.output.capacity();
        self.output.push_str(content);
        self.writes += 1;
        self.growths += usize::from(self.output.capacity() != capacity);
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

impl FlushWriter for EvidenceWriter {
    fn flush(&mut self) -> webui_handler::Result<()> {
        self.flushes += 1;
        Ok(())
    }
}
