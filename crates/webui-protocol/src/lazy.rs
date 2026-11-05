// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fmt::Write as _;

/// Return the compiler-owned, build-local identity of a lazy if body.
///
/// Identities are owner-scoped ordinals. Application code never addresses a
/// region by identity; the client loads it when its condition becomes true.
#[must_use]
pub fn lazy_fragment_id(owner: &str, ordinal: usize) -> String {
    let mut id = String::with_capacity(owner.len() + 16);
    id.push_str(owner);
    id.push_str("::lazy:@");
    let _ = write!(id, "{ordinal}");
    id
}

/// Return the compiler-owned identity of an external block in an owning template.
#[must_use]
pub fn lazy_block_id(owner: &str, index: usize) -> String {
    let mut id = String::with_capacity(owner.len() + 28);
    id.push_str(owner);
    id.push_str("::block:");
    let _ = write!(id, "{index}");
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_are_owner_scoped_and_distinguish_region_roots_from_blocks() {
        assert_eq!(lazy_fragment_id("app-shell", 0), "app-shell::lazy:@0");
        assert_eq!(lazy_fragment_id("app-shell", 12), "app-shell::lazy:@12");
        assert_eq!(lazy_block_id("app-shell", 0), "app-shell::block:0");
    }
}
