// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn config(appearance: &str) -> std::result::Result<DocsConfig, serde_json::Error> {
    let mut value: Value = serde_json::from_str(
        r#"{
        "site": {"title": "Appearance"},
        "basePath": "/docs/",
        "contentDir": ".",
        "nav": [],
        "sidebar": []
    }"#,
    )?;
    value["appearance"] = serde_json::from_str(appearance)?;
    serde_json::from_value(value)
}

#[test]
fn metadata_is_normalized_safe_and_shared_with_ssr() -> TestResult {
    let config = config(
        r#"{
        "defaultTheme": "default",
        "defaultMode": "dark",
        "themes": [
            {"id": "default", "label": "</script><script>bad</script>"},
            {"id": "compact", "label": "Compact"}
        ],
        "storage": {"theme": "mai-design-theme", "mode": "theme"}
    }"#,
    )?;
    let appearance =
        PreparedAppearance::prepare(&config, Path::new("."), &mut StateDependencies::default())?;
    assert!(appearance.has_multiple_themes());
    let state = appearance.state(ShowMode::All);
    assert_eq!(state["theme"], "default");
    assert_eq!(state["mode"], "dark");
    assert_eq!(state["resolvedMode"], "dark");
    assert_eq!(state["themes"][1]["id"], "compact");
    assert!(state.get("tokens").is_none());
    let head = appearance.head_tags(ShowMode::All)?;
    let metadata = head.split("</script>").next().ok_or("missing JSON")?;
    assert!(metadata.contains("\\u003c/script\\u003e"));
    assert!(metadata.contains("\"theme\":\"mai-design-theme\""));
    assert!(!metadata.contains("<script>bad"));
    assert!(appearance.head_tags(ShowMode::Content)?.is_empty());
    assert_eq!(appearance.state(ShowMode::Content)["mode"], "system");
    Ok(())
}

#[test]
fn invalid_configuration_is_actionable() -> TestResult {
    for appearance in [
        r#"{"defaultTheme":"unknown", "themes":[{"id":"a","label":"A"}]}"#,
        r#"{"defaultTheme":"a", "themes":[{"id":"a","label":"A"},{"id":"a","label":"Duplicate"}]}"#,
        r#"{"defaultTheme":"x\"]", "themes":[{"id":"x\"]","label":"Unsafe"}]}"#,
        r#"{"defaultTheme":"a", "themes":[{"id":"a","label":" "}]}"#,
        r#"{"defaultTheme":"a", "themes":[{"id":"a","label":"A"}], "storage":{"theme":"same","mode":"same"}}"#,
    ] {
        let config = config(appearance)?;
        let error =
            PreparedAppearance::prepare(&config, Path::new("."), &mut StateDependencies::default())
                .err()
                .ok_or("expected configuration error")?;
        assert!(error.to_string().contains("help:"));
    }
    let mut config = config(r#"{"defaultTheme":"a", "themes":[{"id":"a","label":"A"}]}"#)?;
    config.theme = Some("./tokens.json".into());
    assert!(PreparedAppearance::prepare(
        &config,
        Path::new("."),
        &mut StateDependencies::default()
    )
    .is_err());
    Ok(())
}

#[test]
fn snapshots_preserve_literals_resets_and_external_input_versions() -> TestResult {
    let dir = tempfile::tempdir()?;
    let source = dir.path().join("family.json");
    fs::write(
        &source,
        r##"{"themes":{"light":{"surface":"#fff","spacing":"initial","alias":"var(--surface)"},"dark":{"surface":"#000","spacing":"12px"}}}"##,
    )?;
    let mut config = config(
        r#"{
        "defaultTheme": "official",
        "themes": [
            {"id":"official", "label":"Official"},
            {"id":"css", "label":"CSS only"}
        ]
    }"#,
    )?;
    config
        .appearance
        .as_mut()
        .ok_or("missing appearance")?
        .themes[0]
        .source = Some(source.to_string_lossy().into_owned());
    let mut dependencies = StateDependencies::default();
    let appearance = PreparedAppearance::prepare(&config, Path::new("."), &mut dependencies)?;
    assert!(appearance.css.contains("--spacing: initial;"));
    assert!(appearance.css.contains("--spacing: 12px;"));
    assert!(appearance.css.contains("--alias: var(--surface);"));
    assert!(appearance.css.contains("--alias: initial;"));
    assert!(appearance
        .css
        .contains(":root:where([data-press-theme=\"css\"])"));
    assert!(appearance.css.contains("prefers-color-scheme: dark"));
    assert_eq!(dependencies.paths, vec![source.canonicalize()?]);
    assert!(dependencies.changed_paths().is_empty());
    let again =
        PreparedAppearance::prepare(&config, Path::new("."), &mut StateDependencies::default())?;
    assert_eq!(appearance.css, again.css);
    let validation = appearance.validation_tokens().ok_or("missing validation")?;
    assert!(validation
        .themes
        .values()
        .all(|tokens| tokens.values().all(String::is_empty)));
    assert!(webui_tokens::validate_required_tokens(&["surface".into()], &validation).is_ok());
    assert!(webui_tokens::validate_required_tokens(&["alias".into()], &validation).is_err());
    fs::write(&source, r#"{"themes":{"light":{}}}"#)?;
    assert_eq!(dependencies.changed_paths(), vec![source.canonicalize()?]);
    let error =
        PreparedAppearance::prepare(&config, Path::new("."), &mut StateDependencies::default())
            .err()
            .ok_or("missing scheme accepted")?;
    assert!(error.to_string().contains("missing its 'dark' map"));
    Ok(())
}

#[test]
fn legacy_and_content_contracts_remain_separate() -> TestResult {
    let dir = tempfile::tempdir()?;
    fs::write(
        dir.path().join("tokens.json"),
        r##"{"themes":{"light":{"color":"#fff"},"dark":{"color":"#000"}}}"##,
    )?;
    let mut config = config("null")?;
    config.theme = Some("./tokens.json".into());
    let legacy =
        PreparedAppearance::prepare(&config, dir.path(), &mut StateDependencies::default())?;
    assert!(legacy.legacy_tokens().is_some());
    assert!(!legacy.has_multiple_themes());
    assert_eq!(legacy.metadata.storage.mode, "webui-press:/docs/:mode");
    config.theme = None;
    config.appearance = serde_json::from_str(
        r#"{
        "defaultTheme":"official", "defaultMode":"dark",
        "themes":[{"id":"official","label":"Official","source":"./tokens.json"},{"id":"css","label":"CSS"}]
    }"#,
    )?;
    config.show = ShowMode::Content;
    let content =
        PreparedAppearance::prepare(&config, dir.path(), &mut StateDependencies::default())?;
    assert!(!content.css.contains("data-theme"));
    assert!(!content.css.contains("data-press-theme"));
    assert!(content.css.contains("--color: #000;"));
    assert!(content.head_tags(config.show)?.is_empty());
    Ok(())
}

#[test]
fn build_emits_shared_artifacts_and_metadata_for_pages_and_404() -> TestResult {
    let dir = tempfile::tempdir()?;
    let content = dir.path().join("content");
    let template = dir.path().join("template");
    fs::create_dir_all(&content)?;
    fs::create_dir_all(&template)?;
    fs::write(
        content.join("index.md"),
        "# Appearance\n\nA complete build.",
    )?;
    fs::write(
        template.join("index.html"),
        r#"<!doctype html><html data-press-theme="{{webuiPress.appearance.theme}}" data-theme-mode="{{webuiPress.appearance.mode}}"><head>{{{headTags}}}</head><body><h1>{{page.title}}</h1>{{{page.content}}}<span>{{webuiPress.appearance.resolvedMode}}</span><output>{{appearance}}</output></body></html>"#,
    )?;
    fs::write(
        dir.path().join("tokens.json"),
        r##"{"themes":{"light":{"surface":"#fff"},"dark":{"surface":"#000"}}}"##,
    )?;
    fs::write(dir.path().join("custom.css"), ":root { --surface: pink; }")?;
    let mut config = config(
        r#"{
        "defaultTheme":"official", "defaultMode":"dark",
        "themes":[{"id":"official","label":"Official","source":"./tokens.json"}]
    }"#,
    )?;
    config.content_dir = content.to_string_lossy().into_owned();
    config.out_dir = dir.path().join("output").to_string_lossy().into_owned();
    config.public_dir = dir.path().join("no-public").to_string_lossy().into_owned();
    config.css = Some(dir.path().join("custom.css").to_string_lossy().into_owned());
    config.state = Some(serde_json::from_str(r#"{"appearance":"subtle"}"#)?);
    crate::build::build_docs(&config, dir.path(), &template)?;
    let output = Path::new(&config.out_dir);
    for path in ["index.html", "404.html"] {
        let html = fs::read_to_string(output.join(path))?;
        assert!(html.contains("data-press-theme=\"official\""));
        assert!(html.contains("data-theme-mode=\"dark\""));
        assert!(html.contains("<span>dark</span>"));
        assert!(html.contains("<output>subtle</output>"));
        let bootstrap = html
            .find("id=\"webui-press-appearance\"")
            .ok_or("missing bootstrap")?;
        let appearance = html
            .find("/docs/appearance.css")
            .ok_or("missing appearance CSS")?;
        let custom = html.find("/docs/theme.css").ok_or("missing consumer CSS")?;
        assert!(bootstrap < appearance && appearance < custom);
        assert!(!html.contains("#fff"));
    }

    assert!(fs::read_to_string(output.join("appearance.css"))?.contains("--surface: #000;"));
    config.show = ShowMode::Content;
    crate::build::build_docs(&config, dir.path(), &template)?;
    for path in ["index.html", "404.html"] {
        let html = fs::read_to_string(output.join(path))?;
        assert!(!html.contains("id=\"webui-press-appearance\""));
        assert!(!html.contains("localStorage"));
        assert!(html.contains("/docs/appearance.css"));
    }
    config.state = Some(serde_json::from_str(
        r#"{"webuiPress":{"appearance":"conflict"}}"#,
    )?);
    let error = crate::build::build_docs(&config, dir.path(), &template)
        .err()
        .ok_or("reserved namespace accepted")?
        .to_string();
    assert!(
        error.contains("webuiPress") && error.contains("reserved"),
        "{error}"
    );
    Ok(())
}

#[test]
fn sourced_families_use_external_css_validation_context() -> TestResult {
    let dir = tempfile::tempdir()?;
    fs::write(
        dir.path().join("tokens.json"),
        r##"{"themes":{"light":{"surface":"#fff"},"dark":{"surface":"#000"}}}"##,
    )?;
    let config = config(
        r#"{"defaultTheme":"official","themes":[{"id":"official","label":"Official","source":"./tokens.json"}]}"#,
    )?;
    let mut appearance =
        PreparedAppearance::prepare(&config, dir.path(), &mut StateDependencies::default())?;
    let required = ["docs-color-bg-alt".into(), "consumer-surface".into()];
    let before = appearance.validation_tokens().ok_or("missing validation")?;
    assert!(webui_tokens::validate_required_tokens(&required, &before).is_err());
    appearance.add_css_validation_context(include_str!("../template/docs.css"))?;
    appearance.add_css_validation_context(":root { --consumer-surface: var(--surface); }")?;
    let validation = appearance.validation_tokens().ok_or("missing validation")?;
    webui_tokens::validate_required_tokens(&required, &validation)?;
    assert!(
        webui_tokens::validate_required_tokens(&["consumer-surfac".into()], &validation).is_err()
    );
    assert!(!appearance.css.contains("--docs-color-bg-alt:"));
    assert!(!appearance.css.contains("--consumer-surface:"));
    assert!(appearance.css.contains("--surface: #fff;"));
    Ok(())
}
