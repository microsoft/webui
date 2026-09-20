// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Shared coordinate clamping between AppKit's `f64` geometry and the
//! cross-platform integer geometry types used by lifecycle events and
//! persisted window state.

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(super) fn clamp_dimension(value: f64) -> u32 {
    value.clamp(0.0, f64::from(u32::MAX)) as u32
}

#[allow(clippy::cast_possible_truncation)]
pub(super) fn clamp_coordinate(value: f64) -> i32 {
    value.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn clamps_out_of_range_dimensions() {
        assert_eq!(clamp_dimension(-5.0), 0);
        assert_eq!(clamp_dimension(f64::from(u32::MAX) + 10.0), u32::MAX);
    }

    #[test]
    fn clamps_out_of_range_coordinates() {
        assert_eq!(clamp_coordinate(f64::from(i32::MIN) - 10.0), i32::MIN);
        assert_eq!(clamp_coordinate(f64::from(i32::MAX) + 10.0), i32::MAX);
    }
}
