// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn region(
    html: Option<&str>,
    html_file: Option<&str>,
    state: Option<Value>,
    state_file: Option<&str>,
) -> RegionConfig {
    RegionConfig {
        html: html.map(str::to_string),
        html_file: html_file.map(str::to_string),
        state,
        state_file: state_file.map(str::to_string),
        script_file: None,
    }
}

fn temp_dir(label: &str) -> Result<PathBuf> {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "webui-press-region-{label}-{}-{id}",
        std::process::id()
    ));
    fs::remove_dir_all(&path).ok();
    fs::create_dir_all(&path).map_err(|error| Error::Io(error.to_string()))?;
    Ok(path)
}

#[test]
fn renders_only_regions_for_the_active_layout() -> TestResult {
    let template = concat!(
        "<main>",
        "<webui-press-region name=\"home.afterHero\" layout=\"home\">",
        "</webui-press-region>",
        "<webui-press-region name=\"shared.footer\" />",
        "</main>"
    );
    let mut configs = BTreeMap::new();
    configs.insert(
        "home.afterHero".to_string(),
        region(Some("<home-card></home-card>"), None, None, None),
    );
    configs.insert(
        "shared.footer".to_string(),
        region(Some("<site-note></site-note>"), None, None, None),
    );
    let regions = RegionSet::load(&configs, Path::new("."), template.to_string())?;

    assert_eq!(
        regions.render("home"),
        "<main><home-card></home-card><site-note></site-note></main>"
    );
    assert_eq!(
        regions.render("doc"),
        "<main><site-note></site-note></main>"
    );
    Ok(())
}

#[test]
fn loads_file_content_and_namespaces_region_state() -> TestResult {
    let dir = temp_dir("files")?;
    fs::write(dir.join("region.html"), "<summary-card></summary-card>")?;
    fs::write(dir.join("state.json"), r#"{"summary":{"title":"Status"}}"#)?;
    let mut configs = BTreeMap::new();
    configs.insert(
        "home.afterHero".to_string(),
        region(None, Some("region.html"), None, Some("state.json")),
    );
    let template = concat!(
        "<webui-press-region name=\"home.afterHero\" layout=\"home\">",
        "</webui-press-region>"
    );
    let regions = RegionSet::load(&configs, &dir, template.to_string())?;
    let mut state = Value::Object(Map::new());
    regions.apply_state("home", &mut state)?;

    assert_eq!(regions.render("home"), "<summary-card></summary-card>");
    assert_eq!(
        state["regions"]["home"]["afterHero"]["summary"]["title"],
        "Status"
    );
    assert_eq!(regions.render("doc"), "");
    assert!(state["regions"]["home"].is_object());

    fs::remove_dir_all(dir).ok();
    Ok(())
}

#[test]
fn rejects_unknown_duplicate_and_nonempty_declarations() {
    let mut configs = BTreeMap::new();
    configs.insert(
        "missing".to_string(),
        region(Some("<p>x</p>"), None, None, None),
    );
    let unknown = RegionSet::load(&configs, Path::new("."), "<main></main>".to_string());
    assert!(matches!(
        unknown,
        Err(Error::Build(message)) if message.contains("does not declare")
    ));

    let duplicate = RegionSet::load(
        &BTreeMap::new(),
        Path::new("."),
        concat!(
            "<webui-press-region name=\"same\" />",
            "<webui-press-region name=\"same\" />"
        )
        .to_string(),
    );
    assert!(matches!(
        duplicate,
        Err(Error::Build(message)) if message.contains("more than once")
    ));

    let nonempty = RegionSet::load(
        &BTreeMap::new(),
        Path::new("."),
        "<webui-press-region name=\"x\"><p>owned</p></webui-press-region>".to_string(),
    );
    assert!(matches!(
        nonempty,
        Err(Error::Build(message)) if message.contains("must be empty")
    ));
}

#[test]
fn ignores_region_text_outside_html_element_contexts() -> TestResult {
    let template = concat!(
        "<!-- <webui-press-region name=\"commented\" /> -->",
        "<script>const marker = '<webui-press-region name=\"scripted\" />';</script>",
        "<style>/* <webui-press-region name=\"styled\" /> */</style>",
        "<div data-marker=\"<webui-press-region name='attribute' />\"></div>",
        "<webui-press-region name=\"active\" />"
    );
    let mut configs = BTreeMap::new();
    configs.insert(
        "active".to_string(),
        region(Some("<p>Active</p>"), None, None, None),
    );
    let regions = RegionSet::load(&configs, Path::new("."), template.to_string())?;
    let rendered = regions.render("doc");

    assert!(rendered.contains("name=\"commented\""));
    assert!(rendered.contains("name=\"scripted\""));
    assert!(rendered.contains("name=\"styled\""));
    assert!(rendered.contains("name='attribute'"));
    assert!(rendered.ends_with("<p>Active</p>"));
    Ok(())
}

#[test]
fn handles_non_ascii_tag_text_without_losing_utf8_boundaries() -> TestResult {
    let template = "<é-tag></é-tag><webui-press-region name=\"active\" />".to_string();
    let mut configs = BTreeMap::new();
    configs.insert(
        "active".to_string(),
        region(Some("<p>Active</p>"), None, None, None),
    );

    let regions = RegionSet::load(&configs, Path::new("."), template)?;
    assert_eq!(regions.render("doc"), "<é-tag></é-tag><p>Active</p>");
    Ok(())
}

#[test]
fn raw_text_scanning_requires_an_exact_closing_tag() -> TestResult {
    let template = concat!(
        "<script>",
        "const falseClose = '</script:x>';",
        "const marker = '<webui-press-region name=\"ignored\" />';",
        "</script>",
        "<webui-press-region name=\"active\" />"
    );
    let mut configs = BTreeMap::new();
    configs.insert(
        "active".to_string(),
        region(Some("<p>Active</p>"), None, None, None),
    );

    let regions = RegionSet::load(&configs, Path::new("."), template.to_string())?;
    let rendered = regions.render("doc");
    assert!(rendered.contains("name=\"ignored\""));
    assert!(rendered.ends_with("<p>Active</p>"));
    Ok(())
}

#[test]
fn ignores_markers_in_all_html_raw_text_contexts() -> TestResult {
    let mut template = String::new();
    for tag in [
        "script", "style", "textarea", "title", "xmp", "iframe", "noembed", "noframes", "noscript",
    ] {
        template.push('<');
        template.push_str(tag);
        template.push('>');
        template.push_str("<webui-press-region name=\"ignored\" />");
        template.push_str("</");
        template.push_str(tag);
        template.push('>');
    }
    template.push_str("<webui-press-region name=\"active\" />");
    let mut configs = BTreeMap::new();
    configs.insert(
        "active".to_string(),
        region(Some("<p>Active</p>"), None, None, None),
    );

    let regions = RegionSet::load(&configs, Path::new("."), template)?;
    let rendered = regions.render("doc");
    assert_eq!(rendered.matches("name=\"ignored\"").count(), 9);
    assert!(rendered.ends_with("<p>Active</p>"));
    Ok(())
}

#[test]
fn plaintext_consumes_marker_text_to_end_of_template() -> TestResult {
    let template = concat!(
        "<plaintext>",
        "<webui-press-region name=\"ignored\" />",
        "<webui-press-region name=\"alsoIgnored\" />"
    );
    let regions = RegionSet::load(&BTreeMap::new(), Path::new("."), template.to_string())?;
    assert_eq!(regions.render("doc"), template);
    Ok(())
}

#[test]
fn rejects_missing_empty_and_duplicate_layout_values() {
    for template in [
        "<webui-press-region name=\"x\" layout />",
        "<webui-press-region name=\"x\" layout=\"\" />",
        "<webui-press-region name=\"x\" layout=\"home\" layout=\"doc\" />",
    ] {
        let result = RegionSet::load(&BTreeMap::new(), Path::new("."), template.to_string());
        assert!(
            matches!(result, Err(Error::Build(message)) if message.contains("layout")),
            "template should reject malformed layout: {template}"
        );
    }
}

#[test]
fn accepts_multiline_declarations_with_trailing_whitespace() -> TestResult {
    let template = concat!(
        "<webui-press-region\n",
        "  name=\"home.panel\"\n",
        "  layout=\"home\"\n",
        "></webui-press-region>"
    );
    let mut configs = BTreeMap::new();
    configs.insert(
        "home.panel".to_string(),
        region(Some("<p>Panel</p>"), None, None, None),
    );

    let regions = RegionSet::load(&configs, Path::new("."), template.to_string())?;
    assert_eq!(regions.render("home"), "<p>Panel</p>");
    Ok(())
}

#[test]
fn rejects_region_names_with_empty_dotted_segments() {
    for name in ["home..afterHero", "home."] {
        let result = RegionSet::load(
            &BTreeMap::new(),
            Path::new("."),
            format!("<webui-press-region name=\"{name}\" />"),
        );
        assert!(
            matches!(result, Err(Error::Build(message)) if message.contains("invalid characters")),
            "name {name:?} should be rejected"
        );
    }
}

#[test]
fn rejects_state_prefix_collisions_but_allows_html_only_prefixes() {
    let template = concat!(
        "<webui-press-region name=\"summary\" />",
        "<webui-press-region name=\"summary.details\" />"
    )
    .to_string();
    let mut stateful = BTreeMap::new();
    stateful.insert(
        "summary".to_string(),
        region(
            Some("<p>Summary</p>"),
            None,
            Some(Value::Object(Map::new())),
            None,
        ),
    );
    stateful.insert(
        "summary.details".to_string(),
        region(
            Some("<p>Details</p>"),
            None,
            Some(Value::Object(Map::new())),
            None,
        ),
    );
    let conflict = RegionSet::load(&stateful, Path::new("."), template.clone());
    assert!(matches!(
        conflict,
        Err(Error::Build(message)) if message.contains("distinct path")
    ));

    let mut html_only = BTreeMap::new();
    html_only.insert(
        "summary".to_string(),
        region(Some("<p>Summary</p>"), None, None, None),
    );
    html_only.insert(
        "summary.details".to_string(),
        region(Some("<p>Details</p>"), None, None, None),
    );
    assert!(RegionSet::load(&html_only, Path::new("."), template).is_ok());
}

#[test]
fn rejects_conflicting_region_sources_and_non_object_state() {
    let template = "<webui-press-region name=\"x\" />".to_string();
    let mut conflicting = BTreeMap::new();
    conflicting.insert(
        "x".to_string(),
        region(Some("<p>x</p>"), Some("x.html"), None, None),
    );
    let conflict = RegionSet::load(&conflicting, Path::new("."), template.clone());
    assert!(matches!(
        conflict,
        Err(Error::Build(message)) if message.contains("mutually exclusive")
    ));

    let mut invalid_state = BTreeMap::new();
    invalid_state.insert(
        "x".to_string(),
        region(Some("<p>x</p>"), None, Some(Value::Array(Vec::new())), None),
    );
    let invalid = RegionSet::load(&invalid_state, Path::new("."), template);
    assert!(matches!(
        invalid,
        Err(Error::Build(message)) if message.contains("must be a JSON object")
    ));
}
