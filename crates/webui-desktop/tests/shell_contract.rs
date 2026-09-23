// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use webui_desktop::{DesktopPackageTarget, DesktopShellConfig};

#[test]
fn shell_only_serializes_implemented_configuration() {
    let shell = DesktopShellConfig::default();
    let value = serde_json::to_value(&shell).unwrap();
    assert_eq!(value, serde_json::json!({ "menus": [] }));
    let decoded: DesktopShellConfig = serde_json::from_value(value).unwrap();
    assert!(decoded.menus.is_empty());
    assert!(decoded.icon_path.is_none());
    assert!(decoded.tray.is_none());
}

#[test]
fn unsupported_shell_fields_fail_instead_of_being_silently_ignored() {
    for field in ["jump_list", "jumpList", "popovers", "downloads"] {
        let error = serde_json::from_value::<DesktopShellConfig>(
            serde_json::json!({ field: { "enabled": true } }),
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown field"));
        assert!(error.to_string().contains(field));
    }
}

#[test]
fn supported_package_targets_round_trip() {
    for (target, name) in [
        (DesktopPackageTarget::MacosApp, "macos-app"),
        (DesktopPackageTarget::WindowsPortable, "windows-portable"),
        (DesktopPackageTarget::LinuxPortable, "linux-portable"),
    ] {
        let json = serde_json::to_value(target).unwrap();
        assert_eq!(json, name);
        assert_eq!(
            serde_json::from_value::<DesktopPackageTarget>(json).unwrap(),
            target
        );
    }
}
