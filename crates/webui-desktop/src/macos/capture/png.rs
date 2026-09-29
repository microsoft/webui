// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Checked PNG from a fixed RGBA bitmap. zlib's public compressBound caps
//! destination allocation *before* encoding; AppKit's PNG encoder does not.

use std::ffi::{c_int, c_uint, c_ulong};

use crate::capture::{CaptureError, MAX_WEB_CAPTURE_PNG_BYTES, MAX_WEB_CAPTURE_RASTER_BYTES};

#[link(name = "z")]
unsafe extern "C" {
    fn compressBound(source_len: c_ulong) -> c_ulong;
    fn compress2(
        destination: *mut u8,
        destination_len: *mut c_ulong,
        source: *const u8,
        source_len: c_ulong,
        level: c_int,
    ) -> c_int;
    fn crc32(checksum: c_ulong, bytes: *const u8, len: c_uint) -> c_ulong;
    #[cfg(test)]
    fn uncompress(
        destination: *mut u8,
        destination_len: *mut c_ulong,
        source: *const u8,
        source_len: c_ulong,
    ) -> c_int;
}

const PNG_FIXED_OVERHEAD: usize = 8 + 25 + 12 + 12;

fn destination_limit(
    source_len: c_ulong,
    max_png_bytes: usize,
) -> Result<(usize, usize), CaptureError> {
    // SAFETY: zlib accepts this checked scalar length and returns its
    // worst-case compressed size without reading a source buffer.
    let compressed_bound = usize::try_from(unsafe { compressBound(source_len) })
        .map_err(|_| CaptureError::TooLarge)?;
    let hard_bound = PNG_FIXED_OVERHEAD
        .checked_add(compressed_bound)
        .ok_or(CaptureError::TooLarge)?;
    if hard_bound > MAX_WEB_CAPTURE_PNG_BYTES {
        return Err(CaptureError::TooLarge);
    }
    let allowed = max_png_bytes
        .checked_sub(PNG_FIXED_OVERHEAD)
        .filter(|bytes| *bytes > 0)
        .ok_or(CaptureError::TooLarge)?;
    Ok((compressed_bound, compressed_bound.min(allowed)))
}

pub(super) fn encode_rgba(
    pixels: &[u8],
    width: u32,
    height: u32,
    max_png_bytes: usize,
) -> Result<Vec<u8>, CaptureError> {
    let row_bytes = usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(4))
        .ok_or(CaptureError::TooLarge)?;
    let pixel_bytes = row_bytes
        .checked_mul(usize::try_from(height).map_err(|_| CaptureError::TooLarge)?)
        .ok_or(CaptureError::TooLarge)?;
    let raw_len = row_bytes
        .checked_add(1)
        .and_then(|row| row.checked_mul(usize::try_from(height).ok()?))
        .ok_or(CaptureError::TooLarge)?;
    if width == 0
        || height == 0
        || pixel_bytes > MAX_WEB_CAPTURE_RASTER_BYTES
        || pixels.len() != pixel_bytes
    {
        return Err(CaptureError::TooLarge);
    }
    let source_len = c_ulong::try_from(raw_len).map_err(|_| CaptureError::TooLarge)?;
    let (_, capacity_limit) = destination_limit(source_len, max_png_bytes)?;

    let mut raw = Vec::with_capacity(raw_len);
    for row in pixels.chunks_exact(row_bytes) {
        raw.push(0); // PNG filter None; one bounded row at a time.
        if row.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 255) {
            raw.extend_from_slice(row);
            continue;
        }
        for pixel in row.as_chunks::<4>().0 {
            let alpha = pixel[3];
            if alpha == 255 {
                raw.extend_from_slice(pixel);
            } else if alpha == 0 {
                raw.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                // AppKit's supported drawing bitmap is premultiplied RGBA.
                // PNG requires unassociated alpha, rounded to nearest.
                for channel in &pixel[..3] {
                    let unpremultiplied =
                        (u32::from(*channel) * 255 + u32::from(alpha) / 2) / u32::from(alpha);
                    raw.push(
                        u8::try_from(unpremultiplied.min(255))
                            .map_err(|_| CaptureError::TooLarge)?,
                    );
                }
                raw.push(alpha);
            }
        }
    }
    let mut capacity = capacity_limit.min(64 * 1024);
    let (mut compressed, encoded_len) = loop {
        let mut buffer = vec![0_u8; capacity];
        let mut written = c_ulong::try_from(buffer.len()).map_err(|_| CaptureError::TooLarge)?;
        // SAFETY: Both owned buffers are live and disjoint; destination
        // capacity never exceeds the checked zlib or caller's byte budget.
        let status = unsafe {
            compress2(
                buffer.as_mut_ptr(),
                &mut written,
                raw.as_ptr(),
                source_len,
                1,
            )
        };
        if status == 0 {
            let written = usize::try_from(written).map_err(|_| CaptureError::TooLarge)?;
            if written > capacity {
                return Err(CaptureError::TooLarge);
            }
            break (buffer, written);
        }
        if status == -5 && capacity == capacity_limit {
            return Err(CaptureError::TooLarge);
        }
        if status != -5 {
            return Err(CaptureError::Native(
                "bounded PNG zlib encoding failed".into(),
            ));
        }
        capacity = capacity.saturating_mul(2).min(capacity_limit);
    };
    let total = PNG_FIXED_OVERHEAD
        .checked_add(encoded_len)
        .ok_or(CaptureError::TooLarge)?;
    if total > max_png_bytes || total > MAX_WEB_CAPTURE_PNG_BYTES {
        return Err(CaptureError::TooLarge);
    }
    compressed.truncate(encoded_len);
    let mut png = Vec::with_capacity(total);
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut header = [0_u8; 13];
    header[0..4].copy_from_slice(&width.to_be_bytes());
    header[4..8].copy_from_slice(&height.to_be_bytes());
    header[8] = 8; // 8-bit RGBA, no interlacing.
    header[9] = 6;
    chunk(&mut png, b"IHDR", &header)?;
    chunk(&mut png, b"IDAT", &compressed)?;
    chunk(&mut png, b"IEND", &[])?;
    if png.len() != total {
        return Err(CaptureError::Incomplete);
    }
    Ok(png)
}

fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) -> Result<(), CaptureError> {
    let length = u32::try_from(data.len()).map_err(|_| CaptureError::TooLarge)?;
    png.extend_from_slice(&length.to_be_bytes());
    let start = png.len();
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    let crc_len = c_uint::try_from(png.len() - start).map_err(|_| CaptureError::TooLarge)?;
    // SAFETY: The borrowed PNG chunk bytes are contiguous and live through
    // this synchronous public zlib CRC call.
    let checksum = unsafe { crc32(0, png[start..].as_ptr(), crc_len) };
    png.extend_from_slice(
        &u32::try_from(checksum)
            .map_err(|_| CaptureError::TooLarge)?
            .to_be_bytes(),
    );
    Ok(())
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn incompressible_maximum_raster_has_preallocated_png_bound_below_twelve_mib() {
        let mut pixels = vec![0; MAX_WEB_CAPTURE_RASTER_BYTES];
        let mut seed = 0x1234_5678_u32;
        for pixel in pixels.as_chunks_mut::<4>().0 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            pixel[..3].copy_from_slice(&seed.to_le_bytes()[..3]);
            pixel[3] = 255;
        }
        let png = encode_rgba(&pixels, 1600, 1200, MAX_WEB_CAPTURE_PNG_BYTES).unwrap();
        assert!(png.len() > 5 * 1024 * 1024);
        assert!(png.len() < MAX_WEB_CAPTURE_PNG_BYTES);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[16..24], &[0, 0, 6, 64, 0, 0, 4, 176]);
        let compressed_len = u32::from_be_bytes(png[33..37].try_into().unwrap()) as usize;
        assert_eq!(&png[37..41], b"IDAT");
        let payload = &png[41..41 + compressed_len];
        let row_bytes = 1600 * 4;
        let mut decoded = vec![0_u8; 1200 * (row_bytes + 1)];
        let mut decoded_len = decoded.len() as c_ulong;
        // SAFETY: Both owned test buffers are live, and the destination
        // exactly holds the expected decompressed scanline length.
        let result = unsafe {
            uncompress(
                decoded.as_mut_ptr(),
                &mut decoded_len,
                payload.as_ptr(),
                payload.len() as c_ulong,
            )
        };
        assert_eq!(result, 0);
        assert_eq!(usize::try_from(decoded_len).unwrap(), decoded.len());
        for (index, row) in pixels.chunks_exact(row_bytes).enumerate() {
            let start = index * (row_bytes + 1);
            assert_eq!(decoded[start], 0);
            assert_eq!(&decoded[start + 1..start + 1 + row_bytes], row);
        }
        assert!(matches!(
            encode_rgba(&pixels, 1600, 1200, 1024),
            Err(CaptureError::TooLarge)
        ));
    }

    #[test]
    fn alpha_and_malformed_buffer_are_explicit() {
        assert!(matches!(
            encode_rgba(&[0; 8], 2, 2, MAX_WEB_CAPTURE_PNG_BYTES),
            Err(CaptureError::TooLarge)
        ));
        let transparent = encode_rgba(&[0, 0, 0, 0], 1, 1, MAX_WEB_CAPTURE_PNG_BYTES).unwrap();
        assert!(transparent.len() < 256);
    }

    #[test]
    fn caller_png_budget_caps_compression_before_destination_allocation() {
        let row_bytes = 64 * 4;
        let raw_len = 64 * (row_bytes + 1);
        let (worst_case, capacity_limit) = destination_limit(raw_len as c_ulong, 1024).unwrap();
        assert!(worst_case > capacity_limit);
        assert_eq!(capacity_limit, 1024 - PNG_FIXED_OVERHEAD);
        assert!(matches!(
            destination_limit(raw_len as c_ulong, PNG_FIXED_OVERHEAD),
            Err(CaptureError::TooLarge)
        ));

        let mut pixels = vec![0_u8; 64 * row_bytes];
        let mut seed = 0x1729_5301_u32;
        for pixel in pixels.as_chunks_mut::<4>().0 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            pixel[..3].copy_from_slice(&seed.to_le_bytes()[..3]);
            pixel[3] = 255;
        }
        assert!(matches!(
            encode_rgba(&pixels, 64, 64, 1024),
            Err(CaptureError::TooLarge)
        ));

        let opaque = [0_u8, 0, 0, 255].repeat(64 * 64);
        let encoded = encode_rgba(&opaque, 64, 64, 1024).unwrap();
        assert!(encoded.len() <= 1024);
    }
}
