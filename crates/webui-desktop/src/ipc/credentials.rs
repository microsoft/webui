// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/// Compare every byte of equal-length credentials without a secret-dependent
/// early exit. Lengths are public and bounded before credential validation.
pub(super) fn constant_equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b)
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}
