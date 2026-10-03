// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use webui_desktop::{bind_owned_local_server, DesktopError};

#[test]
fn owned_binding_rejects_non_loopback_addresses_before_binding() {
    for address in ["0.0.0.0:0", "[::]:0", "192.0.2.1:0"] {
        assert!(matches!(
            bind_owned_local_server(address.parse().unwrap()),
            Err(DesktopError::UnsupportedRuntime { .. })
        ));
    }
}

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
#[test]
fn owned_binding_is_exclusive_and_releases_the_address_on_drop() {
    for address in ["127.0.0.1:0", "[::1]:0"] {
        let listener = bind_owned_local_server(address.parse().unwrap()).unwrap();
        let bound = listener.local_addr().unwrap();
        assert!(bound.ip().is_loopback());
        assert_ne!(bound.port(), 0);
        assert!(bind_owned_local_server(bound).is_err());
        assert!(std::net::TcpListener::bind(bound).is_err());
        drop(listener);
        assert!(bind_owned_local_server(bound).is_ok());
    }
}

#[cfg(any(target_os = "macos", windows))]
#[test]
fn minimal_host_can_configure_packaged_overlay_without_application_ipc() {
    use webui_desktop::{
        DesktopApp, DesktopPlatform, HostLifetime, HttpFrameOrigin, LocalServerOptions,
        LoopbackOrigin, TitlebarStyle, WindowOptions,
    };

    let listener = bind_owned_local_server("127.0.0.1:0".parse().unwrap()).unwrap();
    let address = listener.local_addr().unwrap();
    let origin = LoopbackOrigin::from_socket_addr(address).unwrap();
    let (_owner, lifetime) = HostLifetime::new();
    let window = WindowOptions {
        titlebar: TitlebarStyle::Overlay { height: 64 },
        ..WindowOptions::default()
    };
    let css = webui_desktop::window_css_block(&window, DesktopPlatform::current());
    assert!(css.contains("--webui-titlebar-height:"));
    let frame =
        DesktopApp::from_local_server(LocalServerOptions::new(origin.clone(), lifetime.clone()))
            .window(window.clone())
            .build()
            .unwrap();
    let _grant = frame
        .frame_policy()
        .allow_unprivileged_origin(
            HttpFrameOrigin::from_localhost_subdomain("preview.localhost", address.port()).unwrap(),
        )
        .unwrap();
    let packaged = DesktopApp::from_local_server(LocalServerOptions::new(origin, lifetime))
        .window(window)
        .app_id("com.example.minimal-host")
        .persistent_website_data();
    std::hint::black_box(packaged);
    std::hint::black_box(
        webui_desktop::run_local_server_frame
            as fn(webui_desktop::LocalServerFrame) -> webui_desktop::Result<()>,
    );
}

#[cfg(feature = "native-url-activation")]
#[test]
fn incoming_url_registration_exposes_the_opt_in_typed_contract() {
    use webui_desktop::{
        DesktopApp, HostLifetime, LocalServerOptions, LoopbackOrigin, UrlActivation,
        UrlActivationRegistrationError, MAX_URL_ACTIVATIONS_PER_BATCH, MAX_URL_ACTIVATION_BYTES,
    };

    assert_eq!(MAX_URL_ACTIVATIONS_PER_BATCH, 8);
    assert_eq!(MAX_URL_ACTIVATION_BYTES, 2048);
    let listener = bind_owned_local_server("127.0.0.1:0".parse().unwrap()).unwrap();
    let origin = LoopbackOrigin::from_socket_addr(listener.local_addr().unwrap()).unwrap();
    let (owner, lifetime) = HostLifetime::new();
    let frame =
        DesktopApp::from_local_server(LocalServerOptions::new(origin.clone(), lifetime.clone()))
            .build()
            .unwrap();
    let retired = DesktopApp::from_local_server(LocalServerOptions::new(origin, lifetime))
        .build()
        .unwrap();
    let result = frame.on_url_activation("testapp", |_: UrlActivation| {});
    #[cfg(target_os = "macos")]
    {
        assert!(result.is_ok());
        assert_eq!(
            frame.on_url_activation("testapp", |_| {}),
            Err(UrlActivationRegistrationError::AlreadyRegistered)
        );
        owner.revoke().unwrap();
        assert_eq!(
            retired.on_url_activation("testapp", |_| {}),
            Err(UrlActivationRegistrationError::Closed)
        );
    }
    #[cfg(not(target_os = "macos"))]
    {
        assert_eq!(result, Err(UrlActivationRegistrationError::Unsupported));
        drop((owner, retired));
    }
}
