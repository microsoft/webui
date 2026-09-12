// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use sha2::{Digest, Sha256};
use webui_handler::{FlushWriter, Protocol, RenderOptions, ResponseWriter, WebUIHandler};

use super::Result;

pub(super) const NAMES: &[&str] = &[
    "tiny",
    "small_16k",
    "mixed",
    "raw_1m",
    "attribute_1m",
    "ssr_1000",
    "progressive",
];

pub(super) enum Workload {
    Tiny,
    Small,
    Mixed,
    Raw(String),
    Attribute(String),
    Ssr(Box<(Protocol, serde_json::Value)>),
    Progressive(Box<(Protocol, serde_json::Value)>),
}

impl Workload {
    pub(super) fn new(name: &str) -> Result<Self> {
        Ok(match name {
            "tiny" => Self::Tiny,
            "small_16k" => Self::Small,
            "mixed" => Self::Mixed,
            "raw_1m" => Self::Raw("é🌍".repeat(174_762) + "abcd"),
            "attribute_1m" => Self::Attribute("é🌍".repeat(174_762) + "abcd"),
            "ssr_1000" => {
                let mut state = crate::build_state(1000);
                state["page"] = serde_json::Value::String("contacts".into());
                Self::Ssr(Box::new((crate::build_protocol(), state)))
            }
            "progressive" => Self::Progressive(Box::new(progressive()?)),
            _ => return Err(format!("unknown workload {name}; choose from {NAMES:?}").into()),
        })
    }

    pub(super) fn write(&self, writer: &mut impl FlushWriter) -> webui_handler::Result<()> {
        match self {
            Self::Tiny => {
                for _ in 0..32 {
                    writer.write("12345678")?;
                }
            }
            Self::Small => {
                for _ in 0..1024 {
                    writer.write("0123456789abcdef")?;
                }
            }
            Self::Mixed => mixed(writer)?,
            Self::Raw(value) => writer.write(value)?,
            Self::Attribute(value) => writer.write_attribute("data-value", value)?,
            Self::Ssr(data) => WebUIHandler::new().render(
                &data.0,
                &data.1,
                &RenderOptions::new("index.html", "/contacts"),
                writer,
            )?,
            Self::Progressive(data) => WebUIHandler::new().render_streaming(
                &data.0,
                &data.1,
                &RenderOptions::new("index.html", "/"),
                writer,
            )?,
        }
        writer.end()
    }

    pub(super) fn reference(&self) -> Result<Reference> {
        let mut reference = Reference::default();
        self.write(&mut reference)?;
        reference.flush()?;
        Ok(reference)
    }
}

fn mixed(writer: &mut impl FlushWriter) -> webui_handler::Result<()> {
    writer.write("<!doctype html><html><head><title>Products</title></head><body><main>")?;
    for _ in 0..512 {
        writer.write("<article")?;
        writer.write_attribute("class", "product featured")?;
        writer.write_attribute("data-label", "Café 東京 🌍 &amp; tea")?;
        writer.write_boolean_attribute("hidden")?;
        writer.write("><a")?;
        writer.write_attribute("href", "/products/42?lang=fr&amp;view=card")?;
        writer.write(
            "><h2>Café 東京 🌍</h2><p>Seasonal products shipped worldwide.</p></a></article>",
        )?;
    }
    writer.write("</main></body></html>")
}

fn progressive() -> Result<(Protocol, serde_json::Value)> {
    let mut parser = webui_parser::HtmlParser::new();
    parser.parse(
        "index.html",
        concat!(
            "<html><head><title>Progressive</title></head><body><header>Shell</header>",
            "<boundary name=\"catalog\"><for each=\"item in items\">",
            "<article title=\"{{item}}\">{{item}}</article></for></boundary>",
            "<boundary name=\"details\"><p>{{detail}}</p></boundary>",
            "<footer>Tail</footer></body></html>"
        ),
    )?;
    let protocol = Protocol::new(webui_protocol::WebUIProtocol::new(
        parser.into_fragment_records(),
    ));
    let mut state = serde_json::Map::new();
    state.insert(
        "items".into(),
        serde_json::Value::Array(
            (0..512)
                .map(|_| serde_json::Value::String("Café 東京 🌍".into()))
                .collect(),
        ),
    );
    state.insert(
        "detail".into(),
        serde_json::Value::String("details".repeat(512)),
    );
    Ok((protocol, serde_json::Value::Object(state)))
}

#[derive(Default)]
pub(super) struct Reference {
    pub(super) bytes: Vec<u8>,
    pub(super) flushes: Vec<usize>,
}

impl Reference {
    pub(super) fn sha256(&self) -> String {
        let mut output = String::with_capacity(64);
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for byte in Sha256::digest(&self.bytes) {
            output.push(char::from(HEX[usize::from(byte >> 4)]));
            output.push(char::from(HEX[usize::from(byte & 15)]));
        }
        output
    }
}

impl ResponseWriter for Reference {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.bytes.extend_from_slice(content.as_bytes());
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

impl FlushWriter for Reference {
    fn flush(&mut self) -> webui_handler::Result<()> {
        let end = self.bytes.len();
        if end != 0 && self.flushes.last() != Some(&end) {
            self.flushes.push(end);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_values_and_specialized_attributes_have_fixed_output() -> Result<()> {
        let raw = Workload::new("raw_1m")?.reference()?;
        let attribute = Workload::new("attribute_1m")?.reference()?;
        assert_eq!(raw.bytes.len(), 1024 * 1024);
        assert_eq!(
            attribute.bytes.len(),
            raw.bytes.len() + " data-value=\"\"".len()
        );
        assert_eq!(&attribute.bytes[13..attribute.bytes.len() - 1], &raw.bytes);
        Ok(())
    }

    #[test]
    fn progressive_reference_contains_real_checkpoints_and_terminal() -> Result<()> {
        let reference = Workload::new("progressive")?.reference()?;
        let html = std::str::from_utf8(&reference.bytes)?;
        assert_eq!(html.matches("data-webui-boundary>").count(), 3);
        assert!(html.contains("[2,4,0,{}]"));
        assert!(reference.flushes.len() >= 4);
        assert!(reference.flushes.windows(2).all(|pair| pair[0] < pair[1]));
        Ok(())
    }

    #[test]
    fn ssr_workload_renders_the_large_contacts_route_not_the_dashboard() -> Result<()> {
        let reference = Workload::new("ssr_1000")?.reference()?;
        assert!(reference.bytes.len() > 512 * 1024);
        assert!(std::str::from_utf8(&reference.bytes)?.contains("All Contacts"));
        Ok(())
    }
}
