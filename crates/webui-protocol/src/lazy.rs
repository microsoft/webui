// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fmt::Write as _;

/// Return an owner-scoped resource identity for a lazy if body.
///
/// Authored names are validated by the parser. Anonymous identities are
/// build-local and should not be hardcoded by application code.
#[must_use]
pub fn lazy_fragment_id(owner: &str, name: Option<&str>, ordinal: usize) -> String {
    let mut id = String::with_capacity(owner.len() + name.map_or(24, str::len) + 8);
    id.push_str(owner);
    id.push_str("::lazy:");
    if let Some(name) = name {
        id.push_str(name);
    } else {
        id.push('@');
        let _ = write!(id, "{ordinal}");
    }
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
    fn identities_are_owner_scoped_and_distinguish_anonymous_roots_from_blocks() {
        assert_eq!(
            lazy_fragment_id("app-shell", Some("settings"), 0),
            "app-shell::lazy:settings"
        );
        assert_eq!(lazy_fragment_id("app-shell", None, 0), "app-shell::lazy:@0");
        assert_eq!(lazy_block_id("app-shell", 0), "app-shell::block:0");
    }
}
