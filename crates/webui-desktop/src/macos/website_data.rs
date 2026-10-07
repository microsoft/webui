// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Guard the public app-wide WebKit store against process-name fallback.

use std::path::Path;

use crate::WebsiteDataError;
use objc2_foundation::NSBundle;

pub(crate) fn validate(app_id: Option<&str>) -> Result<(), WebsiteDataError> {
    let id = app_id.ok_or(WebsiteDataError::MissingIdentity)?;
    if !crate::app_identity::valid_app_id(id) {
        return Err(WebsiteDataError::InvalidIdentity);
    }
    let bundle = NSBundle::mainBundle();
    let bundle_id = bundle
        .bundleIdentifier()
        .ok_or(WebsiteDataError::Unpackaged)?;
    let executable = bundle
        .executablePath()
        .ok_or(WebsiteDataError::Unpackaged)?;
    let current_exe = std::env::current_exe().map_err(|_| WebsiteDataError::Unpackaged)?;
    validate_bundle(
        id,
        &bundle_id.to_string(),
        Path::new(&bundle.bundlePath().to_string()),
        Path::new(&executable.to_string()),
        &current_exe,
    )
}

fn validate_bundle(
    app_id: &str,
    bundle_id: &str,
    bundle_path: &Path,
    bundle_executable: &Path,
    current_executable: &Path,
) -> Result<(), WebsiteDataError> {
    if bundle_path
        .extension()
        .is_none_or(|extension| extension != "app")
        || !bundle_path.join("Contents/Info.plist").is_file()
    {
        return Err(WebsiteDataError::Unpackaged);
    }
    let root = bundle_path
        .canonicalize()
        .map_err(|_| WebsiteDataError::Unpackaged)?;
    let executable = bundle_executable
        .canonicalize()
        .map_err(|_| WebsiteDataError::Unpackaged)?;
    let current = current_executable
        .canonicalize()
        .map_err(|_| WebsiteDataError::Unpackaged)?;
    if root.extension().is_none_or(|extension| extension != "app")
        || executable.parent() != Some(root.join("Contents/MacOS").as_path())
        || executable != current
    {
        return Err(WebsiteDataError::Unpackaged);
    }
    if app_id != bundle_id {
        return Err(WebsiteDataError::IdentityMismatch);
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn packaged_identity_requires_exact_main_bundle_and_executable() {
        let temp = tempfile::tempdir().unwrap();
        let app = temp.path().join("Isolated.app");
        let contents = app.join("Contents");
        let executable = contents.join("MacOS/runner");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::write(contents.join("Info.plist"), "fixture").unwrap();
        std::fs::write(&executable, "fixture").unwrap();
        let id = "com.example.isolated";
        assert_eq!(
            validate_bundle(id, id, &app, &executable, &executable),
            Ok(())
        );
        assert_eq!(
            validate_bundle(id, "com.example.other", &app, &executable, &executable),
            Err(WebsiteDataError::IdentityMismatch)
        );
        let other = temp.path().join("runner");
        std::fs::write(&other, "fixture").unwrap();
        assert_eq!(
            validate_bundle(id, id, &app, &executable, &other),
            Err(WebsiteDataError::Unpackaged)
        );
        assert_eq!(
            validate_bundle(id, id, temp.path(), &executable, &executable),
            Err(WebsiteDataError::Unpackaged)
        );
    }
}
