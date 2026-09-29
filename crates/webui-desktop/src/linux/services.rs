// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::path::Path;

use gtk4::gio::{self, prelude::*};

use super::NativeServiceError;

pub(super) fn open_url(url: &str) -> Result<(), NativeServiceError> {
    gio::AppInfo::launch_default_for_uri(url, None::<&gio::AppLaunchContext>)
        .map_err(|error| NativeServiceError::Os(error.to_string()))
}

pub(super) fn open_document(path: &Path) -> Result<(), NativeServiceError> {
    let file = gio::File::for_path(path);
    open_url(file.uri().as_str())
}
