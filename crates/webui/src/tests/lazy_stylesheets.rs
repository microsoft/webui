// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! `lc`: the stylesheet paths a Link build lists beside each lazy pack path,
//! and reports as each pack's `stylesheets`.

use super::*;
use serde_json::Value;

const SHELL_INDEX: &str = "<html><head></head><body><app-shell></app-shell></body></html>";
const ROOT: &str = "app-shell::lazy:@0";
const NESTED: &str = "app-shell::lazy:@1";

/// An owner with an eager badge, a region that adds a form and reuses the
/// badge, and a nested region that adds a picker. Every component is styled.
fn styled_regions_app() -> TempDir {
    create_app_dir(&[
        ("index.html", SHELL_INDEX),
        (
            "app-shell.html",
            concat!(
                "<shared-badge></shared-badge><dialog>",
                r#"<if condition="open" lazy><settings-form></settings-form><shared-badge></shared-badge>"#,
                r#"<if condition="advanced" lazy="demand"><theme-picker></theme-picker></if></if>"#,
                "</dialog>",
            ),
        ),
        ("app-shell.css", ":host { display: block; }"),
        ("shared-badge.html", "<span>Badge</span>"),
        ("shared-badge.css", "span { color: blue; }"),
        ("settings-form.html", "<form></form>"),
        ("settings-form.css", "form { display: grid; }"),
        ("theme-picker.html", "<select></select>"),
        ("theme-picker.css", "select { width: 100%; }"),
    ])
}

fn options(app: &Path, css: CssStrategy) -> BuildOptions {
    BuildOptions {
        plugin: Some(Plugin::WebUI),
        css,
        ..default_options(app)
    }
}

fn metadata(result: &BuildResult, id: &str) -> Value {
    let json = result.protocol.components.get(id).map_or_else(
        || &result.protocol.client_fragments[id].template_json,
        |component| &component.template_json,
    );
    serde_json::from_str(json).unwrap()
}

fn pack<'a>(result: &'a BuildResult, region: &str) -> &'a LazyPackFile {
    result
        .lazy_pack_files
        .iter()
        .find(|pack| pack.region == region)
        .unwrap()
}

/// The `href` of each link resource a pack registers, in the order its
/// closures first name them.
fn registered_stylesheets(pack: &LazyPackFile) -> Vec<String> {
    let content: Value = serde_json::from_str(&pack.content).unwrap();
    let styles = &content["componentStyles"];
    let mut hrefs: Vec<String> = Vec::new();
    let Some(closures) = styles["closures"].as_object() else {
        return hrefs;
    };
    for names in closures.values() {
        for name in names.as_array().unwrap() {
            let Some(href) = styles["resources"][name.as_str().unwrap()]["href"].as_str() else {
                continue;
            };
            if !hrefs.iter().any(|listed| listed == href) {
                hrefs.push(href.to_owned());
            }
        }
    }
    hrefs
}

fn templates(result: &BuildResult) -> impl Iterator<Item = &str> {
    result
        .protocol
        .components
        .values()
        .map(|component| component.template_json.as_str())
        .chain(
            result
                .protocol
                .client_fragments
                .values()
                .map(|fragment| fragment.template_json.as_str()),
        )
}

#[test]
fn link_builds_list_each_regions_stylesheets_where_its_condition_lives() {
    let app = styled_regions_app();
    let result = build(options(app.path(), CssStrategy::Link)).unwrap();

    let owner = metadata(&result, "app-shell");
    assert_eq!(owner["lp"][ROOT], pack(&result, ROOT).path());
    // The owner already loads the badge, so only the form's stylesheet is new.
    assert_eq!(
        owner["lc"],
        serde_json::json!({ ROOT: ["settings-form.css"] })
    );
    assert_eq!(
        registered_stylesheets(pack(&result, ROOT)),
        ["settings-form.css"]
    );

    // The nested region is listed in the template that holds its condition.
    let region = metadata(&result, ROOT);
    assert_eq!(region["lp"][NESTED], pack(&result, NESTED).path());
    assert_eq!(
        region["lc"],
        serde_json::json!({ NESTED: ["theme-picker.css"] })
    );
    assert_eq!(
        registered_stylesheets(pack(&result, NESTED)),
        ["theme-picker.css"]
    );
    assert!(owner["lc"].get(NESTED).is_none(), "{owner}");

    // The outer pack serializes the region template with its nested `lc`.
    let outer: Value = serde_json::from_str(&pack(&result, ROOT).content).unwrap();
    assert_eq!(outer["templates"][ROOT]["lc"], region["lc"]);

    // The pack report lists the same paths.
    assert_eq!(pack(&result, ROOT).stylesheets, ["settings-form.css"]);
    assert_eq!(pack(&result, NESTED).stylesheets, ["theme-picker.css"]);

    for (name, _) in &result.css_files {
        assert!(!name.contains('/'), "stylesheet paths are base-relative");
    }
}

#[test]
fn bundled_link_builds_list_the_chunks_each_pack_registers() {
    // A Shadow sheet whose tree holds two Light controls: bundling merges the
    // controls into one chunk that the region's pack registers.
    let app = create_app_dir(&[
        ("index.html", SHELL_INDEX),
        (
            "app-shell.html",
            r#"<if condition="open" lazy><settings-sheet></settings-sheet></if>"#,
        ),
        (
            "settings-sheet.html",
            "<form-alpha></form-alpha><form-beta></form-beta>",
        ),
        ("settings-sheet.css", ":host { display: block; }"),
        ("form-alpha.html", "<template><span>A</span></template>"),
        ("form-alpha.css", ".alpha { color: red; }"),
        ("form-beta.html", "<template><span>B</span></template>"),
        ("form-beta.css", ".beta { color: green; }"),
    ]);
    let mut options = options(app.path(), CssStrategy::Link);
    options.css_bundle = true;
    let result = build(options).unwrap();

    let listed: Vec<String> =
        serde_json::from_value(metadata(&result, "app-shell")["lc"][ROOT].clone()).unwrap();
    assert_eq!(listed, registered_stylesheets(pack(&result, ROOT)));
    assert_eq!(listed, pack(&result, ROOT).stylesheets);
    let merged = result
        .protocol
        .style_chunks
        .iter()
        .find(|chunk| chunk.component_tags.len() > 1)
        .expect("the controls share one chunk");
    assert!(
        listed.contains(&merged.css_href),
        "{listed:?} must name the merged chunk {}",
        merged.css_href
    );
    for href in &listed {
        assert!(
            result.css_files.iter().any(|(name, _)| name == href),
            "{href} must be an emitted file"
        );
    }
}

#[test]
fn style_and_module_builds_carry_css_in_packs_and_list_no_stylesheets() {
    let app = styled_regions_app();
    for css in [CssStrategy::Style, CssStrategy::Module] {
        let mut options = options(app.path(), css);
        options.metafile = true;
        let result = build(options).unwrap();
        assert!(pack(&result, ROOT)
            .content
            .contains("form { display: grid; }"));
        for json in templates(&result) {
            assert!(!json.contains(r#""lc""#), "{json}");
        }
        assert!(metadata(&result, "app-shell")["lp"].get(ROOT).is_some());
        // The report keeps the field, empty, like `scripts`.
        assert!(result
            .lazy_pack_files
            .iter()
            .all(|pack| pack.stylesheets.is_empty()));
        let report: Value = serde_json::from_str(result.metafile.as_deref().unwrap()).unwrap();
        for entry in report["lazyPacks"].as_array().unwrap() {
            assert_eq!(entry["stylesheets"], serde_json::json!([]), "{entry}");
        }
    }
}

#[test]
fn metafile_reports_the_stylesheets_each_region_loads() {
    let app = styled_regions_app();
    let mut options = options(app.path(), CssStrategy::Link);
    options.metafile = true;
    let result = build(options).unwrap();
    let report: Value = serde_json::from_str(result.metafile.as_deref().unwrap()).unwrap();
    let entries = report["lazyPacks"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    for entry in entries {
        let region = entry["region"].as_str().unwrap();
        let pack = pack(&result, region);
        assert_eq!(entry["stylesheets"], serde_json::json!(pack.stylesheets));
        // The same list the template holding the region's condition carries.
        let holder = if region == ROOT { "app-shell" } else { ROOT };
        assert_eq!(
            metadata(&result, holder)["lc"][region],
            entry["stylesheets"]
        );
    }
    let outer = entries
        .iter()
        .find(|entry| entry["region"] == ROOT)
        .unwrap();
    assert_eq!(
        outer["stylesheets"],
        serde_json::json!(["settings-form.css"])
    );
}

#[test]
fn regions_without_stylesheets_are_left_out_and_lc_is_omitted_when_empty() {
    let app = create_app_dir(&[
        ("index.html", SHELL_INDEX),
        (
            "app-shell.html",
            concat!(
                r#"<if condition="open" lazy><plain-note></plain-note>"#,
                r#"<if condition="advanced" lazy><theme-picker></theme-picker></if></if>"#,
            ),
        ),
        ("plain-note.html", "<p>Note</p>"),
        ("theme-picker.html", "<select></select>"),
        ("theme-picker.css", "select { width: 100%; }"),
    ]);
    let result = build(options(app.path(), CssStrategy::Link)).unwrap();
    let owner = metadata(&result, "app-shell");
    assert!(owner["lp"].get(ROOT).is_some());
    assert!(owner.get("lc").is_none(), "{owner}");
    assert_eq!(
        metadata(&result, ROOT)["lc"],
        serde_json::json!({ NESTED: ["theme-picker.css"] })
    );
}

#[test]
fn stylesheet_lists_are_deterministic() {
    let app = styled_regions_app();
    let first = build(options(app.path(), CssStrategy::Link)).unwrap();
    let second = build(options(app.path(), CssStrategy::Link)).unwrap();
    assert_eq!(first.lazy_pack_files, second.lazy_pack_files);
    assert_eq!(first.protocol.build_id, second.protocol.build_id);
    let mut left: Vec<&str> = templates(&first).collect();
    let mut right: Vec<&str> = templates(&second).collect();
    left.sort_unstable();
    right.sort_unstable();
    assert_eq!(left, right);
}
