// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Shared path-safe identity rule for native app-owned storage.
pub(crate) fn valid_app_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 255
        && !id.ends_with('.')
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}
