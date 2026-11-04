// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::{HandlerError, ResponseWriter, Result};

// Substitute only the literal compiler-owned FAST Link placeholder, not authored HTML.
pub(crate) struct TemplateAssetWriter<'a> {
    pub(crate) writer: &'a mut dyn ResponseWriter,
    pub(crate) base: Option<&'a str>,
}

impl ResponseWriter for TemplateAssetWriter<'_> {
    fn write(&mut self, content: &str) -> Result<()> {
        let Some(base) = self.base else {
            return self.writer.write(content);
        };
        let Some((prefix, href_and_tail)) = content.split_once(" data-webui-stylesheet href=\"")
        else {
            return self.writer.write(content);
        };
        let (href, _) = href_and_tail
            .split_once('"')
            .ok_or_else(invalid_placeholder)?;
        self.writer.write(prefix)?;
        self.writer.write(" data-webui-stylesheet href=\"")?;
        if crate::is_base_relative(href) {
            self.writer.write_escaped(base)?;
        }
        self.writer.write(href_and_tail)
    }

    fn end(&mut self) -> Result<()> {
        self.writer.end()
    }
}

#[cold]
#[inline(never)]
fn invalid_placeholder() -> HandlerError {
    HandlerError::Invariant(
        "invalid compiler-owned FAST stylesheet placeholder; rebuild the protocol".into(),
    )
}
