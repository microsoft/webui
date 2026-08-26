// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;

#[test]
fn property_bindings_populate_fast_component_render_scope() {
    let app = create_app_dir(&[
        (
            "index.html",
            r#"<html><body><todo-app :rendertodos="{{todos}}"></todo-app></body></html>"#,
        ),
        (
            "todo-app.template.html",
            r#"<f-template name="todo-app"><template><ul><f-repeat value="{{todo in rendertodos}}"><todo-item :todo="{{todo}}"></todo-item></f-repeat></ul></template></f-template>"#,
        ),
        (
            "todo-item.template.html",
            r#"<f-template name="todo-item"><template><li data-id="{{todo.id}}">{{todo.title}}</li></template></f-template>"#,
        ),
    ]);
    let mut options = default_options(app.path());
    options.plugin = Some(Plugin::FastV3);
    let result = build(options).unwrap();
    let protocol = Protocol::new(result.protocol);
    let handler = WebUIHandler::with_plugin(|| {
        Box::new(webui_handler::plugin::fast_v3::FastV3HydrationPlugin::new())
    });
    let todos: Vec<_> = (0..100)
        .map(|index| {
            serde_json::json!({
                "id": format!("todo-{index}"),
                "title": format!("Todo {index}"),
            })
        })
        .collect();
    let state = serde_json::json!({ "todos": todos });
    let mut writer = StringWriter { buf: String::new() };

    handler
        .render(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();

    let ssr = writer.buf.split("<f-template").next().unwrap_or_default();
    assert_eq!(ssr.matches(r#"<li data-id="todo-"#).count(), 100);
    assert!(!ssr.contains(":rendertodos"));
    assert!(!ssr.contains(":todo"));
}

#[test]
fn build_discovers_generated_local_template() {
    let app = create_app_dir(&[
        ("index.html", "<custom-button></custom-button>"),
        (
            "custom-button.template.html",
            r#"<f-template><template><button>{{label}}</button></template></f-template>"#,
        ),
        ("custom-button.styles.css", "button { color: red; }"),
    ]);
    let mut options = default_options(app.path());
    options.plugin = Some(Plugin::FastV3);
    options.css = CssStrategy::Style;

    let result = build(options).unwrap();
    let component = &result.protocol.components["custom-button"];
    assert!(component.template.contains("<button>{{label}}</button>"));
    assert!(component
        .template
        .contains("<style>button { color: red; }</style>"));
}

#[test]
fn build_discovers_fast_npm_package_layout() {
    let project = TempDir::new().unwrap();
    let app = project.path().join("src");
    let package = project.path().join("node_modules").join("custom");
    fs::create_dir_all(package.join("components/button")).unwrap();
    fs::create_dir_all(package.join("components/item")).unwrap();
    fs::create_dir_all(package.join("components/textarea")).unwrap();
    fs::create_dir_all(&app).unwrap();
    fs::write(
        app.join("index.html"),
        "<custom-button></custom-button><custom-item></custom-item><custom-textarea></custom-textarea>",
    )
    .unwrap();
    fs::write(
        package.join("package.json"),
        r#"{
            "name": "custom",
            "customElements": "./custom-elements.json",
            "exports": { ".": "./index.js" }
        }"#,
    )
    .unwrap();
    fs::write(
        package.join("custom-elements.json"),
        r#"{
            "schemaVersion": "1.0.0",
            "modules": [{
                "kind": "javascript-module",
                "path": "components/button/button.js",
                "declarations": [{
                    "kind": "class",
                    "name": "Button",
                    "tagName": "custom-button"
                }]
            }, {
                "kind": "javascript-module",
                "path": "components/special-item/special-item.js",
                "declarations": [{
                    "kind": "class",
                    "name": "SpecialItem",
                    "tagName": "custom-special-item"
                }]
            }, {
                "kind": "javascript-module",
                "path": "components/text-area/text-area.js",
                "declarations": [{
                    "kind": "class",
                    "name": "TextArea",
                    "tagName": "custom-text-area"
                }]
            }]
        }"#,
    )
    .unwrap();
    fs::write(
        package.join("components/button/button.template.html"),
        r#"<f-template name="custom-button"><template><button>{{label}}</button></template></f-template>"#,
    )
    .unwrap();
    fs::write(
        package.join("components/button/button.styles.css"),
        "button { color: blue; }",
    )
    .unwrap();
    fs::write(
        package.join("components/item/item.template.html"),
        r#"<f-template name="custom-item"><template><span>{{value}}</span></template></f-template>"#,
    )
    .unwrap();
    fs::write(
        package.join("components/textarea/textarea.template.html"),
        r#"<f-template name="custom-textarea"><template><textarea>{{value}}</textarea></template></f-template>"#,
    )
    .unwrap();

    let mut options = default_options(&app);
    options.plugin = Some(Plugin::FastV3);
    options.components = vec!["custom".to_string()];
    options.css = CssStrategy::Style;

    let result = build(options).unwrap();
    let component = &result.protocol.components["custom-button"];
    assert!(component.template.contains("<button>{{label}}</button>"));
    assert!(component
        .template
        .contains("<style>button { color: blue; }</style>"));
    assert!(result.protocol.components["custom-item"]
        .template
        .contains("<span>{{value}}</span>"));
    assert!(result.protocol.components["custom-textarea"]
        .template
        .contains("<textarea>{{value}}</textarea>"));
}

#[test]
fn css_public_base_keeps_shadow_template_styled() {
    let app = create_app_dir(&[
        ("index.html", "<my-card>Hello</my-card>"),
        (
            "my-card.html",
            r#"<template shadowrootmode="open"><div><slot></slot></div></template>"#,
        ),
        ("my-card.css", ".card { color: red; }"),
    ]);
    let mut options = default_options(app.path());
    options.plugin = Some(Plugin::FastV3);
    options.css_file_name_template = "[name]-[hash].[ext]".to_string();
    options.css_public_base = Some("https://cdn.example.com/assets".to_string());
    let result = build(options).unwrap();

    let filename = &result.css_files[0].0;
    let expected_href = format!("https://cdn.example.com/assets/{filename}");
    let template = &result.protocol.components["my-card"].template;
    assert_eq!(
        result.protocol.component_style_resource("my-card"),
        Some(expected_href.as_str())
    );
    assert!(template.contains(&expected_href));
}

#[test]
fn authored_template_light_build_is_rejected() {
    let app = create_app_dir(&[
        ("index.html", "<my-card>Hello</my-card>"),
        (
            "my-card.template.html",
            r#"<f-template><template><div>card</div></template></f-template>"#,
        ),
        ("my-card.css", ".card { color: red; }"),
    ]);
    let mut options = default_options(app.path());
    options.dom = DomStrategy::Light;
    options.plugin = Some(Plugin::FastV3);
    options.css_file_name_template = "[name]-[hash].[ext]".to_string();
    options.css_public_base = Some("https://cdn.example.com/assets".to_string());
    let error = build(options).expect_err("FAST Light build must fail");
    assert!(matches!(
        error,
        WebUIError::Parse {
            source: webui_parser::ParserError::Template(ref diagnostic),
            ..
        } if diagnostic.error_code()
            == Some(webui_parser::codes::FAST_LIGHT_DOM_UNSUPPORTED)
    ));
}

#[test]
fn authored_template_dom_diagnostic_uses_original_location() {
    let app = create_app_dir(&[
        ("index.html", "<my-card></my-card>"),
        (
            "my-card.template.html",
            "<f-template\n  name=\"my-card\"\n  shadowrootmode=\"closed\">\n  <template><div>card</div></template>\n</f-template>",
        ),
    ]);
    let mut options = default_options(app.path());
    options.plugin = Some(Plugin::FastV3);

    let error = build(options).expect_err("closed FAST Shadow root must fail");
    assert!(matches!(
        error,
        WebUIError::Parse {
            source: webui_parser::ParserError::Template(ref diagnostic),
            ..
        } if diagnostic.error_code() == Some(webui_parser::codes::INVALID_SHADOW_ROOT_MODE)
            && diagnostic.position_line_column() == Some((3, 3))
            && diagnostic.snippet_text() == Some("shadowrootmode=\"closed\">")
    ));
}

#[test]
fn authored_template_duplicate_shadow_diagnostic_uses_second_location() {
    let app = create_app_dir(&[
        ("index.html", "<my-card></my-card>"),
        (
            "my-card.template.html",
            "<f-template name=\"my-card\" shadowrootmode=\"open\">\n  <template shadowrootmode=\"open\"><div>card</div></template>\n</f-template>",
        ),
    ]);
    let mut options = default_options(app.path());
    options.plugin = Some(Plugin::FastV3);

    let error = build(options).expect_err("duplicate Shadow mode must fail");
    assert!(matches!(
        error,
        WebUIError::Parse {
            source: webui_parser::ParserError::Template(ref diagnostic),
            ..
        } if diagnostic.error_code() == Some(webui_parser::codes::INVALID_SHADOW_ROOT_MODE)
            && diagnostic.position_line_column() == Some((2, 13))
    ));
}

#[test]
fn authored_template_slot_diagnostic_uses_original_location() {
    let app = create_app_dir(&[
        ("index.html", "<my-card></my-card>"),
        (
            "my-card.template.html",
            "<f-template name=\"my-card\">\n  <template>\n    <slot name=\"label\"></slot>\n  </template>\n</f-template>",
        ),
    ]);
    let mut options = default_options(app.path());
    options.plugin = Some(Plugin::FastV3);
    options.dom = DomStrategy::Light;

    let error = build(options).expect_err("Light DOM slot must fail");
    assert!(matches!(
        error,
        WebUIError::Parse {
            source: webui_parser::ParserError::Template(ref diagnostic),
            ..
        } if diagnostic.error_code() == Some(webui_parser::codes::LIGHT_DOM_SLOT)
            && diagnostic.position_line_column() == Some((3, 5))
            && diagnostic.snippet_text() == Some("<slot name=\"label\"></slot>")
    ));
}

#[test]
fn build_selects_fast_v3() {
    let app = create_app_dir(&[("index.html", "<h1>Hello</h1>")]);
    let mut options = default_options(app.path());
    options.plugin = Some(Plugin::FastV3);

    let result = build(options).unwrap();
    assert!(result.protocol.fragments.contains_key("index.html"));
    assert_eq!(
        result.protocol.initial_state_strategy,
        webui_protocol::InitialStateStrategy::Full as i32
    );
}

#[test]
fn variants_build_authored_templates() {
    let app = create_app_dir(&[
        (
            "index.html",
            "<named-card></named-card><fallback-card></fallback-card><plain-card></plain-card>",
        ),
        (
            "file-card.html",
            r#"<f-template name="named-card"><template><f-when value="{{visible}}"><f-repeat value="{{item in items}}"><button @click="{save()}">{{item.label}}</button></f-repeat></f-when></template></f-template>"#,
        ),
        ("file-card.css", "/* remove */ .card { color: red; }"),
        (
            "fallback-card.html",
            r#"<f-template><template><span>{{label}}</span></template></f-template>"#,
        ),
        (
            "plain-card.html",
            r#"<if condition="visible"><span>{{label}}</span></if>"#,
        ),
    ]);

    for plugin in [Plugin::Fast, Plugin::FastV2, Plugin::FastV3] {
        let mut options = default_options(app.path());
        options.plugin = Some(plugin);
        options.css = CssStrategy::Style;

        let result = build(options).unwrap();
        assert!(result.protocol.fragments.contains_key("named-card"));
        let component = result
            .protocol
            .components
            .get("named-card")
            .expect("named FAST component");
        assert!(component.template.contains("<f-when"));
        assert!(component.template.contains("<f-repeat"));
        assert!(component.template.contains(r#"@click="{save()}""#));
        assert!(component
            .template
            .contains("<style>.card { color: red; }</style>"));
        let fallback = result
            .protocol
            .components
            .get("fallback-card")
            .expect("file-named FAST component");
        assert!(fallback
            .template
            .contains(r#"<f-template name="fallback-card" shadowrootmode="open">"#));
        assert!(fallback.template.contains("<span>{{label}}</span>"));
        let plain = result
            .protocol
            .components
            .get("plain-card")
            .expect("ordinary WebUI component");
        assert!(plain
            .template
            .contains(r#"<f-template name="plain-card" shadowrootmode="open">"#));
        assert!(plain.template.contains(r#"<f-when value="{{visible}}">"#));
    }
}

#[test]
fn authored_artifact_is_not_double_wrapped() {
    for css in [CssStrategy::Style, CssStrategy::Module, CssStrategy::Link] {
        let app = create_app_dir(&[
            ("index.html", "<named-card></named-card>"),
            (
                "file-card.html",
                concat!(
                    r#"<f-template name="named-card">"#,
                    "<!-- lead comment -->",
                    r#"<template><button @click="{save()}" :config="{config}" ?disabled="{{disabled}}" f-ref="{button}">{{label}}</button></template>"#,
                    "<!-- tail comment -->",
                    r#"</f-template>"#,
                ),
            ),
            ("file-card.css", ".card { color: red; }"),
        ]);

        let mut options = default_options(app.path());
        options.plugin = Some(Plugin::FastV3);
        options.css = css;

        let result =
            build(options).unwrap_or_else(|error| panic!("build failed for {css:?}: {error}"));
        let template = &result.protocol.components["named-card"].template;
        assert_eq!(template.matches("<template").count(), 1);
        assert!(template.contains(r#"<f-template name="named-card" shadowrootmode="open">"#));
        for binding in [
            r#"@click="{save()}""#,
            r#":config="{config}""#,
            r#"?disabled="{{disabled}}""#,
            r#"f-ref="{button}""#,
        ] {
            assert!(template.contains(binding));
        }
        assert!(!template.contains("lead comment"));
        assert!(!template.contains("tail comment"));
    }
}

#[test]
fn build_preserves_structured_diagnostic() {
    let app = create_app_dir(&[
        ("index.html", "<bad-card></bad-card>"),
        (
            "bad-card.html",
            "<f-template name=\"bad-card\">\n  <template>\n    <f-choose></f-choose>\n  </template>\n</f-template>",
        ),
    ]);
    let mut options = default_options(app.path());
    options.plugin = Some(Plugin::FastV3);

    let err = build(options).expect_err("invalid FAST component should fail the build");
    let WebUIError::ComponentRegistration {
        source: ParserError::Template(diag),
        ..
    } = err
    else {
        panic!("expected a structured component diagnostic, got {err:?}");
    };
    assert_eq!(diag.error_code(), Some("invalid-fast-template"));
    assert_eq!(diag.component_name(), Some("bad-card"));
    assert_eq!(diag.position_line_column(), Some((3, 5)));
    assert_eq!(diag.snippet_text(), Some("<f-choose>"));
    assert!(diag.help_text().is_some());
}
