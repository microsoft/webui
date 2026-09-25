// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);
type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn region(html: &str) -> RegionConfig {
    RegionConfig {
        html: Some(html.to_string()),
        html_file: None,
        state: None,
        state_file: None,
        script_file: None,
    }
}

fn temp_dir() -> Result<PathBuf> {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("webui-press-region-{}-{id}", std::process::id()));
    fs::remove_dir_all(&path).ok();
    fs::create_dir_all(&path).map_err(|error| Error::Io(error.to_string()))?;
    Ok(path)
}

#[test]
fn renders_layout_scoped_and_global_regions() -> TestResult {
    let template = concat!(
        "<main>",
        "<webui-press-region name = \"home.panel\" layout = \"home\"><p>Default</p></webui-press-region>",
        "<webui-press-region name=\"home.default\" layout=\"home\"><h1>After Hero</h1></webui-press-region>",
        "<webui-press-region name=\"shared.footer\" />",
        "<webui-press-region name=\"optional\" />",
        "</main>"
    );
    let configs = BTreeMap::from([
        ("home.panel".to_string(), region("<home-card></home-card>")),
        (
            "shared.footer".to_string(),
            region("<site-note></site-note>"),
        ),
    ]);
    let regions = RegionSet::load(&configs, Path::new("."), template.to_string())?;

    assert_eq!(
        regions.render("home"),
        "<main><home-card></home-card><h1>After Hero</h1><site-note></site-note></main>"
    );
    assert_eq!(
        regions.render("doc"),
        "<main><site-note></site-note></main>"
    );
    assert_eq!(regions.template_shell(), "<main></main>");
    Ok(())
}

#[test]
fn bundled_template_exposes_stable_regions() -> TestResult {
    let template = include_str!("../../template/index.html");
    let regions = RegionSet::load(&BTreeMap::new(), Path::new("."), template.to_string())?;
    let names: Vec<&str> = regions
        .regions
        .iter()
        .map(|region| region.name.as_str())
        .collect();

    assert_eq!(
        names,
        [
            "site.navigation",
            "site.announcement",
            "home.hero",
            "home.afterHero",
            "home.features",
            "home.footer",
            "doc.sidebar",
            "doc.context",
            "doc.beforeContent",
            "page.beforeContent",
            "full.beforeContent",
            "doc.afterContent",
            "page.afterContent",
            "full.afterContent",
            "doc.pageNavigation",
            "doc.footer",
            "page.footer",
        ]
    );
    assert!(regions
        .regions
        .iter()
        .find(|region| region.name == "home.hero")
        .and_then(|region| region.html.as_deref())
        .is_some_and(|html| html.contains("home-hero")));
    Ok(())
}

#[test]
fn loads_files_and_namespaces_state() -> TestResult {
    let dir = temp_dir()?;
    fs::write(dir.join("region.html"), "<summary-card></summary-card>")?;
    fs::write(dir.join("state.json"), r#"{"title":"Status"}"#)?;
    let configs = BTreeMap::from([(
        "home.panel".to_string(),
        RegionConfig {
            html: None,
            html_file: Some("region.html".to_string()),
            state: None,
            state_file: Some("state.json".to_string()),
            script_file: Some("region.ts".to_string()),
        },
    )]);
    let template = "<webui-press-region name=\"home.panel\" layout=\"home\"></webui-press-region>";
    let regions = RegionSet::load(&configs, &dir, template.to_string())?;
    let mut state = Value::Object(Map::new());
    regions.apply_state("home", &mut state)?;

    assert_eq!(regions.render("home"), "<summary-card></summary-card>");
    assert_eq!(state["regions"]["home"]["panel"]["title"], "Status");
    assert_eq!(
        regions.script_files("home").collect::<Vec<_>>(),
        ["region.ts"]
    );
    assert_eq!(regions.script_files("doc").count(), 0);

    fs::remove_dir_all(dir).ok();
    Ok(())
}

#[test]
fn state_only_config_keeps_default_html() -> TestResult {
    let template =
        "<webui-press-region name=\"summary\"><summary-card></summary-card></webui-press-region>";
    let mut config = region("");
    config.html = None;
    config.state = Some(Value::Object(Map::from_iter([(
        "title".to_string(),
        Value::String("Status".to_string()),
    )])));
    let regions = RegionSet::load(
        &BTreeMap::from([("summary".to_string(), config)]),
        Path::new("."),
        template.to_string(),
    )?;

    assert_eq!(regions.render("doc"), "<summary-card></summary-card>");
    Ok(())
}

#[test]
fn rejects_invalid_declarations() {
    let cases = [
        (
            concat!(
                "<webui-press-region name=\"same\" />",
                "<webui-press-region name=\"same\" />"
            ),
            "more than once",
        ),
        (
            "<webui-press-region name=\"x\" layout />",
            "requires a quoted value",
        ),
        (
            "<webui-press-region name=\"x\" layout=home />",
            "requires a quoted value",
        ),
        (
            "<webui-press-region name=\"x\" layout=\"\" />",
            "requires a non-empty value",
        ),
        (
            "<webui-press-region name=\"x\" layout=\"home\" layout=\"doc\" />",
            "declared more than once",
        ),
        (
            "<webui-press-region name=\"x\" unexpected=\"value\" />",
            "unsupported attribute",
        ),
        (
            concat!(
                "<webui-press-region name=\"outer\">",
                "<webui-press-region name=\"inner\"></webui-press-region>",
                "</webui-press-region>"
            ),
            "cannot be nested",
        ),
    ];
    for (template, expected) in cases {
        let result = RegionSet::load(&BTreeMap::new(), Path::new("."), template.to_string());
        assert!(
            matches!(result, Err(Error::Build(message)) if message.contains(expected)),
            "expected {expected:?} for {template:?}"
        );
    }
}

#[test]
fn ignores_commented_markers() -> TestResult {
    let template = concat!(
        "<!-- <webui-press-region name=\"ignored\" /> -->",
        "<webui-press-region name=\"active\">Active</webui-press-region>"
    );
    let regions = RegionSet::load(&BTreeMap::new(), Path::new("."), template.to_string())?;

    assert_eq!(
        regions.render("doc"),
        "<!-- <webui-press-region name=\"ignored\" /> -->Active"
    );
    Ok(())
}

#[test]
fn rejects_undeclared_config_and_invalid_names() {
    let configs = BTreeMap::from([("missing".to_string(), region("<p>x</p>"))]);
    assert!(matches!(
        RegionSet::load(&configs, Path::new("."), "<main></main>".to_string()),
        Err(Error::Build(message)) if message.contains("does not declare")
    ));

    for name in ["home..panel", "home."] {
        let template = format!("<webui-press-region name=\"{name}\" />");
        assert!(matches!(
            RegionSet::load(&BTreeMap::new(), Path::new("."), template),
            Err(Error::Build(message)) if message.contains("invalid characters")
        ));
    }
}

#[test]
fn rejects_state_prefix_collisions_but_allows_html_only_prefixes() {
    let template = concat!(
        "<webui-press-region name=\"summary\" />",
        "<webui-press-region name=\"summary.details\" />"
    )
    .to_string();
    let mut parent = region("<p>Summary</p>");
    parent.state = Some(Value::Object(Map::new()));
    let mut child = region("<p>Details</p>");
    child.state = Some(Value::Object(Map::new()));
    let stateful = BTreeMap::from([
        ("summary".to_string(), parent),
        ("summary.details".to_string(), child),
    ]);
    assert!(matches!(
        RegionSet::load(&stateful, Path::new("."), template.clone()),
        Err(Error::Build(message)) if message.contains("distinct path")
    ));

    let html_only = BTreeMap::from([
        ("summary".to_string(), region("<p>Summary</p>")),
        ("summary.details".to_string(), region("<p>Details</p>")),
    ]);
    assert!(RegionSet::load(&html_only, Path::new("."), template).is_ok());
}

#[test]
fn rejects_conflicting_sources_and_non_object_state() {
    let template = "<webui-press-region name=\"x\" />".to_string();
    let mut conflicting = region("<p>x</p>");
    conflicting.html_file = Some("x.html".to_string());
    assert!(matches!(
        RegionSet::load(
            &BTreeMap::from([("x".to_string(), conflicting)]),
            Path::new("."),
            template.clone()
        ),
        Err(Error::Build(message)) if message.contains("mutually exclusive")
    ));

    let mut invalid_state = region("<p>x</p>");
    invalid_state.state = Some(Value::Array(Vec::new()));
    assert!(matches!(
        RegionSet::load(
            &BTreeMap::from([("x".to_string(), invalid_state)]),
            Path::new("."),
            template
        ),
        Err(Error::Build(message)) if message.contains("must be a JSON object")
    ));
}
