// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Transfer sizes for the lazy-region cost report.
//!
//! Only builds that request a metafile measure packs: brotli at quality 11 is
//! slow, and the size is a report, not build output.

use std::io::{self, Write};

use brotli::enc::BrotliEncoderParams;
use rayon::prelude::*;
use webui_handler::lazy_packs::LazyPackFile;

use crate::WebUIError;

/// Brotli quality of the reported size: the setting static hosts use to
/// precompress files.
const BROTLI_QUALITY: i32 = 11;

/// Record each pack's brotli size in [`LazyPackFile::brotli_bytes`].
///
/// # Errors
///
/// Returns [`WebUIError::Serialization`] if the encoder fails.
pub(crate) fn measure(packs: &mut [LazyPackFile]) -> Result<(), WebUIError> {
    packs.par_iter_mut().try_for_each(|pack| {
        pack.brotli_bytes = Some(brotli_size(pack.content.as_bytes())?);
        Ok(())
    })
}

fn brotli_size(content: &[u8]) -> Result<usize, WebUIError> {
    let params = BrotliEncoderParams {
        quality: BROTLI_QUALITY,
        size_hint: content.len(),
        ..BrotliEncoderParams::default()
    };
    let mut counter = ByteCounter(0);
    brotli::BrotliCompress(&mut &content[..], &mut counter, &params)
        .map_err(|error| encoder_error(&error))?;
    Ok(counter.0)
}

/// Counts compressed bytes without keeping them.
struct ByteCounter(usize);

impl Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cold]
#[inline(never)]
fn encoder_error(error: &io::Error) -> WebUIError {
    WebUIError::Serialization(format!(
        "Failed to measure the brotli size of a lazy pack: {error}"
    ))
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use webui_handler::lazy_packs::LazyTrigger;

    #[test]
    fn brotli_sizes_match_a_full_compression() {
        let content =
            r#"{"type":"webui-pack","templates":{"settings-panel":{"h":"<p>Settings</p>"}}}"#
                .repeat(20);
        let mut compressed = Vec::new();
        brotli::BrotliCompress(
            &mut content.as_bytes(),
            &mut compressed,
            &BrotliEncoderParams {
                quality: 11,
                size_hint: content.len(),
                ..BrotliEncoderParams::default()
            },
        )
        .unwrap();
        let mut decoded = Vec::new();
        brotli::BrotliDecompress(&mut compressed.as_slice(), &mut decoded).unwrap();
        assert_eq!(decoded, content.as_bytes());

        let mut packs = [LazyPackFile {
            file_name: "app-shell.0123456789abcdef.json".to_owned(),
            content,
            region: "app-shell::lazy:@0".to_owned(),
            owner: "app-shell".to_owned(),
            trigger: LazyTrigger::Idle,
            components: Vec::new(),
            scripts: Vec::new(),
            brotli_bytes: None,
        }];
        measure(&mut packs).unwrap();
        assert_eq!(packs[0].brotli_bytes, Some(compressed.len()));
        assert!(compressed.len() < packs[0].content.len());
    }
}
