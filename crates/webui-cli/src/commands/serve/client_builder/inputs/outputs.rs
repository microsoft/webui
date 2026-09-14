// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;
use webui_discovery::{DiscoveryPlugin, WebUIDiscoveryPlugin};

#[test]
fn compiler_discoverable_app_output_never_reuses_ssr() -> Result<()> {
    for name in ["generated", "dist", "target"] {
        let mut fixture = Fixture::new()?;
        fixture.output = fixture.config.app_dir.join(name);
        fs::create_dir(&fixture.output)?;
        let inputs = fixture.inputs()?;
        assert!(inputs.capture()?.is_none(), "{name}");
        fs::write(fixture.output.join("copied-card.html"), "<p>copied</p>")?;
        fs::write(fixture.output.join("copied-card.css"), "p { color: red }")?;
        let discovered = WebUIDiscoveryPlugin::new().discover_local(&fixture.config.app_dir)?;
        assert!(discovered
            .iter()
            .any(|component| component.tag_name == "copied-card"));
        assert!(inputs.capture()?.is_none(), "{name}");
        fs::write(fixture.output.join("copied-card.css"), "p { color: blue }")?;
        assert!(inputs.capture()?.is_none(), "{name}");
    }
    Ok(())
}

#[test]
fn compiler_discoverable_component_root_output_never_reuses_ssr() -> Result<()> {
    let mut fixture = Fixture::new()?;
    // Ignored ancestors cannot hide an explicitly supplied discovery root.
    let components = fixture.sibling("node_modules").join("local-components");
    fixture.output = components.join("generated");
    fs::create_dir_all(&fixture.output)?;
    fixture
        .config
        .app_args
        .components
        .push(components.to_string_lossy().into_owned());
    assert!(fixture.inputs()?.capture()?.is_none());
    Ok(())
}

#[test]
fn sdk_hidden_output_and_sibling_dist_remain_cacheable() -> Result<()> {
    for relative in [
        PathBuf::from("node_modules")
            .join(".cache")
            .join("webui-dev")
            .join("run"),
        PathBuf::from(".cache").join("webui-dev").join("run"),
        PathBuf::from("assets").join(".generated"),
    ] {
        let mut fixture = Fixture::new()?;
        snapshot(&fixture.inputs()?)?;
        fixture.output = fixture.config.app_dir.join(relative);
        fs::create_dir_all(&fixture.output)?;
        let inputs = fixture.inputs()?;
        let before = snapshot(&inputs)?;
        fs::write(
            fixture.output.join("copied-card.html"),
            "<p>not discoverable</p>",
        )?;
        fs::write(fixture.output.join("bundle.js"), "generated")?;
        let discovered = WebUIDiscoveryPlugin::new().discover_local(&fixture.config.app_dir)?;
        assert!(!discovered
            .iter()
            .any(|component| component.tag_name == "copied-card"));
        assert_eq!(before, snapshot(&inputs)?);
    }
    Ok(())
}

#[test]
fn generated_explicit_state_and_theme_inside_sdk_hidden_output_are_hashed() -> Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.output = fixture
        .config
        .app_dir
        .join("node_modules")
        .join(".cache")
        .join("webui-dev")
        .join("run");
    fs::create_dir_all(&fixture.output)?;
    let state = fixture.output.join("state.json");
    let theme = fixture.output.join("theme.json");
    fs::write(&state, "{}")?;
    fs::write(&theme, "{}")?;
    fixture.config.state_file = Some(state.clone());
    let inputs = Inputs::new(
        &fixture.config,
        Some(theme.to_str().context("theme path")?),
        &fixture.output,
    )?;
    let before = snapshot(&inputs)?;
    fs::write(&state, r#"{"value":1}"#)?;
    let state_changed = snapshot(&inputs)?;
    assert_ne!(before, state_changed);
    fs::write(&theme, r#"{"brand":"red"}"#)?;
    assert_ne!(state_changed, snapshot(&inputs)?);
    Ok(())
}
