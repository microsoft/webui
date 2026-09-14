// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;
use crate::commands::common::{
    AppArgs, CssStrategy, DomStrategy, LegalComments, DEFAULT_ASSET_FILE_NAME_TEMPLATE,
};

struct Fixture {
    _directory: tempfile::TempDir,
    config: RenderConfig,
    output: PathBuf,
}

impl Fixture {
    fn new() -> Result<Self> {
        // Keep test artifacts inside the checkout, including on Windows.
        let directory = tempfile::Builder::new()
            .prefix(".ssr-inputs-")
            .tempdir_in(std::env::current_dir()?)?;
        let app = directory.path().join("app");
        let output = directory.path().join("dist");
        fs::create_dir(&app)?;
        fs::create_dir_all(&output)?;
        fs::write(app.join("index.html"), "<sample-card></sample-card>")?;
        fs::write(app.join("sample-card.html"), "<p>Hello</p>")?;
        fs::write(app.join("sample-card.css"), "p { color: red; }")?;
        fs::write(app.join("sample-card.ts"), "export {};")?;
        let config = RenderConfig {
            app_args: AppArgs {
                app: app.clone(),
                entry: "index.html".to_owned(),
                css: CssStrategy::Link,
                dom: DomStrategy::Shadow,
                css_bundle: false,
                plugin: Some(webui::Plugin::WebUI),
                components: Vec::new(),
                projection_manifests: Vec::new(),
                asset_file_name_template: DEFAULT_ASSET_FILE_NAME_TEMPLATE.to_owned(),
                css_public_base: None,
                legal_comments: LegalComments::Inline,
            },
            app_dir: app,
            state_file: None,
            token_file: None,
            component_asset_roots: Vec::new(),
            metafile: None,
            base_path: None,
        };
        Ok(Self {
            _directory: directory,
            config,
            output,
        })
    }

    fn inputs(&self) -> Result<Inputs> {
        Inputs::new(&self.config, None, &self.output)
    }

    fn sibling(&self, name: &str) -> PathBuf {
        self._directory.path().join(name)
    }
}

fn snapshot(inputs: &Inputs) -> Result<Snapshot> {
    inputs
        .capture()?
        .context("Expected a complete SSR snapshot")
}

#[test]
fn unchanged_snapshots_and_standard_script_content_reuse() -> Result<()> {
    let fixture = Fixture::new()?;
    let inputs = fixture.inputs()?;
    let before = snapshot(&inputs)?;
    assert_eq!(before, snapshot(&inputs)?);
    for extension in ["ts", "js", "mjs", "cjs", "mts", "cts", "jsx", "tsx"] {
        let script = fixture
            .config
            .app_dir
            .join("sample-card")
            .with_extension(extension);
        fs::write(&script, "initial")?;
        let before_edit = snapshot(&inputs)?;
        fs::write(&script, "completely different and longer")?;
        assert_eq!(before_edit, snapshot(&inputs)?);
    }
    Ok(())
}

#[test]
fn script_addition_removal_and_rename_invalidate() -> Result<()> {
    let fixture = Fixture::new()?;
    let inputs = fixture.inputs()?;
    let before = snapshot(&inputs)?;
    let script = fixture.config.app_dir.join("other-card.js");
    fs::write(&script, "export {};")?;
    let added = snapshot(&inputs)?;
    assert_ne!(before, added);
    let renamed = script.with_file_name("renamed-card.js");
    fs::rename(&script, &renamed)?;
    assert_ne!(added, snapshot(&inputs)?);
    fs::remove_file(&renamed)?;
    assert_eq!(before, snapshot(&inputs)?);
    fs::remove_file(fixture.config.app_dir.join("sample-card.ts"))?;
    assert_ne!(before, snapshot(&inputs)?);
    Ok(())
}

#[test]
fn non_script_contents_including_package_metadata_invalidate() -> Result<()> {
    let fixture = Fixture::new()?;
    let inputs = fixture.inputs()?;
    for name in [
        "index.html",
        "sample-card.css",
        "package.json",
        "unknown.bin",
    ] {
        let path = fixture.config.app_dir.join(name);
        fs::write(&path, "before")?;
        let before = snapshot(&inputs)?;
        fs::write(&path, "after!")?;
        assert_ne!(before, snapshot(&inputs)?, "{name}");
    }
    Ok(())
}

#[test]
fn generated_output_is_not_an_input_even_when_new_files_appear() -> Result<()> {
    let fixture = Fixture::new()?;
    let inputs = fixture.inputs()?;
    let before = snapshot(&inputs)?;
    fs::create_dir_all(fixture.output.join("chunks"))?;
    fs::write(fixture.output.join("bundle.js"), "one")?;
    fs::write(fixture.output.join("chunks").join("large.css"), "styles")?;
    assert_eq!(before, snapshot(&inputs)?);
    fs::write(fixture.output.join("bundle.js"), "two")?;
    fs::remove_file(fixture.output.join("chunks").join("large.css"))?;
    assert_eq!(before, snapshot(&inputs)?);
    Ok(())
}

#[test]
fn explicit_generated_state_and_theme_are_required_and_hashed() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let state = fixture.output.join("state.json");
    let theme = fixture.output.join("theme.json");
    fixture.config.state_file = Some(state.clone());
    let inputs = Inputs::new(
        &fixture.config,
        Some(theme.to_str().context("theme path")?),
        &fixture.output,
    )?;
    assert!(inputs.capture()?.is_none());
    fs::write(&state, "{}")?;
    assert!(inputs.capture()?.is_none());
    fs::write(&theme, r#"{"brand":"red"}"#)?;
    let before = snapshot(&inputs)?;
    fs::write(&state, r#"{"value":1}"#)?;
    let changed_state = snapshot(&inputs)?;
    assert_ne!(before, changed_state);
    fs::write(&theme, r#"{"brand":"blue"}"#)?;
    assert_ne!(changed_state, snapshot(&inputs)?);
    fs::remove_file(&state)?;
    assert!(inputs.capture()?.is_none());
    fs::write(&state, "{}")?;
    fs::remove_file(&theme)?;
    assert!(inputs.capture()?.is_none());
    Ok(())
}

#[test]
fn external_state_theme_entry_and_local_components_are_included() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let components = fixture.sibling("components");
    fs::create_dir(&components)?;
    let external_html = components.join("external-card.html");
    fs::write(&external_html, "external")?;
    fixture
        .config
        .app_args
        .components
        .push(components.to_string_lossy().into_owned());
    let state = fixture.sibling("state.json");
    let theme = fixture.sibling("theme.json");
    let entry = fixture.sibling("entry.html");
    fs::write(&state, "{}")?;
    fs::write(&theme, "{}")?;
    fs::write(&entry, "<p>entry</p>")?;
    fixture.config.state_file = Some(state.clone());
    fixture.config.app_args.entry = entry.to_string_lossy().into_owned();
    let inputs = Inputs::new(
        &fixture.config,
        Some(theme.to_str().context("theme path")?),
        &fixture.output,
    )?;
    for path in [&state, &theme, &entry, &external_html] {
        let before = snapshot(&inputs)?;
        fs::write(path, "changed")?;
        assert_ne!(before, snapshot(&inputs)?);
    }
    let before = snapshot(&inputs)?;
    fs::write(fixture.sibling("builder-only.ts"), "not an SSR input")?;
    assert_eq!(before, snapshot(&inputs)?);
    Ok(())
}

#[test]
fn npm_theme_is_resolved_again_on_every_capture() -> Result<()> {
    let fixture = Fixture::new()?;
    let outer = fixture.sibling("node_modules").join("theme");
    fs::create_dir_all(&outer)?;
    fs::write(outer.join("tokens.json"), "{}")?;
    let inputs = Inputs::new(&fixture.config, Some("theme"), &fixture.output)?;
    let before = snapshot(&inputs)?;
    fs::write(outer.join("tokens.json"), r#"{"brand":"red"}"#)?;
    let changed = snapshot(&inputs)?;
    assert_ne!(before, changed);
    let nearer = fixture.config.app_dir.join("node_modules").join("theme");
    fs::create_dir_all(&nearer)?;
    fs::write(nearer.join("tokens.json"), r#"{"brand":"red"}"#)?;
    assert_ne!(changed, snapshot(&inputs)?);
    Ok(())
}

#[test]
fn deleted_required_inputs_and_unresolved_roots_never_cache() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let inputs = fixture.inputs()?;
    snapshot(&inputs)?;
    fs::remove_file(fixture.config.app_dir.join("index.html"))?;
    assert!(inputs.capture()?.is_none());
    fs::write(fixture.config.app_dir.join("index.html"), "restored")?;
    snapshot(&inputs)?;
    let missing = fixture.sibling("missing");
    fixture
        .config
        .app_args
        .components
        .push(missing.to_string_lossy().into_owned());
    assert!(fixture.inputs()?.capture()?.is_none());
    fs::create_dir(&missing)?;
    snapshot(&fixture.inputs()?)?;
    fs::remove_dir(&missing)?;
    assert!(fixture.inputs()?.capture()?.is_none());
    Ok(())
}

#[test]
fn oversized_non_script_inputs_never_cache() -> Result<()> {
    let fixture = Fixture::new()?;
    let inputs = fixture.inputs()?;
    for name in ["oversized.txt", "oversized.html", "oversized.css"] {
        let path = fixture.config.app_dir.join(name);
        File::create(&path)?.set_len(MAX_BYTES as u64 + 1)?;
        assert!(inputs.capture()?.is_none());
        fs::remove_file(&path)?;
        snapshot(&inputs)?;
    }
    Ok(())
}

#[test]
fn ignored_descendants_do_not_hide_explicit_roots_under_target() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let app = fixture.sibling("target").join("nested").join("app");
    fs::create_dir_all(&app)?;
    fs::write(app.join("index.html"), "app")?;
    fixture.config.app_dir = app.clone();
    fixture.config.app_args.app = app.clone();
    let inputs = fixture.inputs()?;
    let before = snapshot(&inputs)?;
    for ignored in ["node_modules", ".git", ".hidden"] {
        fs::create_dir(app.join(ignored))?;
        fs::write(app.join(ignored).join("ignored.html"), "ignore")?;
    }
    assert_eq!(before, snapshot(&inputs)?);
    fs::write(app.join("index.html"), "changed")?;
    assert_ne!(before, snapshot(&inputs)?);
    Ok(())
}

#[test]
fn creation_order_does_not_change_snapshot_ordering() -> Result<()> {
    let fixture = Fixture::new()?;
    let inputs = fixture.inputs()?;
    let names = ["z-last", "a-first", "m-middle"];
    for name in names {
        let directory = fixture.config.app_dir.join(name);
        fs::create_dir(&directory)?;
        fs::write(directory.join("file.txt"), name)?;
    }
    let before = snapshot(&inputs)?;
    for name in names {
        fs::remove_dir_all(fixture.config.app_dir.join(name))?;
    }
    for name in names.into_iter().rev() {
        let directory = fixture.config.app_dir.join(name);
        fs::create_dir(&directory)?;
        fs::write(directory.join("file.txt"), name)?;
    }
    assert_eq!(before, snapshot(&inputs)?);
    Ok(())
}

#[test]
fn immutable_configuration_participates_in_the_snapshot() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let old_inputs = fixture.inputs()?;
    let before = snapshot(&old_inputs)?;
    fixture.config.app_args.dom = DomStrategy::Light;
    assert_ne!(before, snapshot(&fixture.inputs()?)?);
    assert_eq!(before, snapshot(&old_inputs)?);
    Ok(())
}

#[test]
fn token_map_insertion_order_does_not_change_the_configuration_hash() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let names = ["dark", "light"];
    let values = [("brand", "red"), ("text", "black")];
    fixture.config.token_file = Some(webui::TokenFile {
        themes: names
            .iter()
            .map(|name| {
                (
                    (*name).to_owned(),
                    values
                        .iter()
                        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                        .collect(),
                )
            })
            .collect(),
    });
    let before = snapshot(&fixture.inputs()?)?;
    fixture.config.token_file = Some(webui::TokenFile {
        themes: names
            .iter()
            .rev()
            .map(|name| {
                (
                    (*name).to_owned(),
                    values
                        .iter()
                        .rev()
                        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                        .collect(),
                )
            })
            .collect(),
    });
    assert_eq!(before, snapshot(&fixture.inputs()?)?);
    Ok(())
}

#[test]
fn reusable_standard_script_edit_preserves_compiled_protocol() -> Result<()> {
    let fixture = Fixture::new()?;
    let inputs = fixture.inputs()?;
    let before = snapshot(&inputs)?;
    let original = webui::build(
        fixture
            .config
            .app_args
            .to_build_options(&fixture.config.app_dir),
    )?;
    fs::write(
        fixture.config.app_dir.join("sample-card.ts"),
        "export const browserOnly = 'changed';",
    )?;
    assert_eq!(before, snapshot(&inputs)?);
    let rebuilt = webui::build(
        fixture
            .config
            .app_args
            .to_build_options(&fixture.config.app_dir),
    )?;
    assert_eq!(original.protocol, rebuilt.protocol);
    Ok(())
}

#[test]
fn projection_fast_and_unresolved_package_closures_always_prepare() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let manifest = fixture.output.join("projection.json");
    fixture
        .config
        .app_args
        .projection_manifests
        .push(manifest.clone());
    let projection = fixture.inputs()?;
    assert!(projection.capture()?.is_none());
    fs::write(&manifest, "{}")?;
    assert!(projection.capture()?.is_none());
    fs::write(&manifest, r#"{"changed":true}"#)?;
    assert!(projection.capture()?.is_none());
    fixture.config.app_args.projection_manifests.clear();
    for plugin in [
        webui::Plugin::Fast,
        webui::Plugin::FastV2,
        webui::Plugin::FastV3,
    ] {
        fixture.config.app_args.plugin = Some(plugin);
        assert!(fixture.inputs()?.capture()?.is_none());
    }
    fixture.config.app_args.plugin = Some(webui::Plugin::WebUI);
    fixture
        .config
        .app_args
        .components
        .push("@scope/package".to_owned());
    assert!(fixture.inputs()?.capture()?.is_none());
    Ok(())
}

#[path = "streams.rs"]
mod streams;

#[path = "filesystem.rs"]
mod filesystem;

#[path = "outputs.rs"]
mod outputs;

#[path = "discovery.rs"]
mod discovery;
