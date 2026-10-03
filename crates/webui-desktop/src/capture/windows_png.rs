// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Windows viewport admission and PNG header checks without COM dependencies.

use super::{CaptureError, CaptureOptions, MAX_WEB_CAPTURE_RASTER_BYTES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Viewport {
    width: u32,
    height: u32,
}

// The adapter verifies BoundsMode=RAW_PIXELS and that the controller bounds
// equal the content HWND's client rect before using these raw screen pixels.
pub(crate) fn preflight(
    width: u32,
    height: u32,
    options: CaptureOptions,
) -> Result<Viewport, CaptureError> {
    if width == 0 || height == 0 {
        return Err(CaptureError::Incomplete);
    }
    let raster = usize::try_from(width)
        .ok()
        .and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h)))
        .and_then(|pixels| pixels.checked_mul(4));
    if width > options.max_width
        || height > options.max_height
        || raster.is_none_or(|n| n > MAX_WEB_CAPTURE_RASTER_BYTES)
    {
        return Err(CaptureError::TooLarge);
    }
    Ok(Viewport { width, height })
}

pub(crate) fn validate_png(
    png: &[u8],
    viewport: Viewport,
    options: CaptureOptions,
) -> Result<(u32, u32), CaptureError> {
    if png.len() < 45
        || !png.starts_with(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR")
        || !png.ends_with(b"\0\0\0\0IEND\xaeB`\x82")
    {
        return Err(CaptureError::Incomplete);
    }
    let width = u32::from_be_bytes(
        png[16..20]
            .try_into()
            .map_err(|_| CaptureError::Incomplete)?,
    );
    let height = u32::from_be_bytes(
        png[20..24]
            .try_into()
            .map_err(|_| CaptureError::Incomplete)?,
    );
    let raster = usize::try_from(width)
        .ok()
        .and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h)))
        .and_then(|pixels| pixels.checked_mul(4));
    if width == 0
        || height == 0
        || width > options.max_width
        || height > options.max_height
        || raster.is_none_or(|n| n > MAX_WEB_CAPTURE_RASTER_BYTES)
    {
        return Err(CaptureError::TooLarge);
    }
    // Reject a crop or unaccounted-for enlargement. One pixel on either
    // axis accommodates edge rounding; never waive physical options above.
    if width.abs_diff(viewport.width) > 1 || height.abs_diff(viewport.height) > 1 {
        return Err(CaptureError::Incomplete);
    }
    Ok((width, height))
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&width.to_be_bytes());
        png.extend_from_slice(&height.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        png.extend_from_slice(&[0; 4]); // Synthetic IHDR CRC (header-only policy).
        png.extend_from_slice(b"\0\0\0\0IEND\xaeB`\x82");
        png
    }

    #[test]
    fn raw_pixel_bounds_do_not_grow_with_monitor_dpi_or_rasterization_scale() {
        let options = CaptureOptions::new();
        // Both measurements are raw physical pixels even when the controller
        // rasterizes CSS at 200% DPI; DPI must not double either viewport.
        assert!(preflight(900, 700, options).is_ok());
        assert_eq!(preflight(1800, 1400, options), Err(CaptureError::TooLarge));
        assert_eq!(preflight(0, 700, options), Err(CaptureError::Incomplete));
    }

    #[test]
    fn raw_pixel_png_uses_measured_ihdr_not_dpi_scaled_client_dimensions() {
        let options = CaptureOptions::new();
        let viewport = preflight(900, 700, options).unwrap();
        let png = png_header(900, 700);
        assert_eq!(validate_png(&png, viewport, options), Ok((900, 700)));
        assert_eq!(
            validate_png(&png_header(899, 700), viewport, options),
            Ok((899, 700))
        );
        assert_eq!(
            validate_png(&png_header(1800, 1400), viewport, options),
            Err(CaptureError::TooLarge)
        );
        assert_eq!(
            validate_png(&png_header(1000, 700), viewport, options),
            Err(CaptureError::Incomplete)
        );
        assert_eq!(
            validate_png(&png_header(901, 700), viewport, options),
            Ok((901, 700))
        );
        assert_eq!(
            validate_png(&png_header(350, 250), viewport, options),
            Err(CaptureError::Incomplete)
        );
        let selected = CaptureOptions::new().max_dimensions(900, 700).unwrap();
        assert_eq!(
            validate_png(&png_header(901, 700), viewport, selected),
            Err(CaptureError::TooLarge)
        );
    }

    #[test]
    fn rejects_header_shorter_than_a_complete_ihdr_and_iend() {
        let options = CaptureOptions::new();
        let viewport = preflight(900, 700, options).unwrap();
        let mut png = png_header(900, 700);
        png.drain(24..33);
        assert_eq!(png.len(), 36);
        assert_eq!(
            validate_png(&png, viewport, options),
            Err(CaptureError::Incomplete)
        );
    }
}
