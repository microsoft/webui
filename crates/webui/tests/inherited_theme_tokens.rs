// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use std::collections::HashMap;
use std::fs;
use webui::{build, BuildOptions, CssStrategy, DomStrategy, Plugin};
use webui_tokens::TokenFile;

#[test]
fn light_component_selector_default_satisfies_inherited_usage() {
    let app = tempfile::tempdir().unwrap();
    for (name, content) in [
        (
            "index.html",
            "<!doctype html><html lang=\"en\"><head><title>Local token validation reproduction</title></head><body><local-token></local-token></body></html>",
        ),
        ("local-token.html", "<span>Inherited local color</span>"),
        (
            "local-token.css",
            "local-token {\n  --private-color: #123456;\n}\nlocal-token span {\n  color: var(--private-color);\n}",
        ),
    ] {
        fs::write(app.path().join(name), content).unwrap();
    }

    let options = || BuildOptions {
        app_dir: app.path().to_path_buf(),
        dom: DomStrategy::Light,
        css: CssStrategy::Style,
        plugin: Some(Plugin::WebUI),
        ..BuildOptions::default()
    };
    let without_theme = build(options()).unwrap();
    let with_theme = build(BuildOptions {
        theme: Some(TokenFile {
            themes: HashMap::from([(
                "light".to_string(),
                HashMap::from([("brand".to_string(), "#654321".to_string())]),
            )]),
        }),
        ..options()
    })
    .expect("an inherited component-local property must not require a theme token");
    assert!(with_theme.protocol.tokens.is_empty());
    assert!(with_theme.warnings.is_empty());
    assert_eq!(with_theme.protocol, without_theme.protocol);
}

#[test]
fn stylesheet_inheritance_is_independent_of_dom_mode_and_css_delivery() {
    let app = tempfile::tempdir().unwrap();
    for (name, content) in [
        ("index.html", "<local-token></local-token>"),
        (
            "local-token.html",
            "<div class=\"scope\"><span>Local</span></div>",
        ),
        (
            "local-token.css",
            ".scope { --local: red; } .scope span { color: var(--local); }",
        ),
    ] {
        fs::write(app.path().join(name), content).unwrap();
    }
    for dom in [DomStrategy::Light, DomStrategy::Shadow] {
        for css in [CssStrategy::Link, CssStrategy::Style, CssStrategy::Module] {
            let result = build(BuildOptions {
                app_dir: app.path().to_path_buf(),
                dom,
                css,
                theme: Some(TokenFile {
                    themes: HashMap::from([("light".to_string(), HashMap::new())]),
                }),
                plugin: Some(Plugin::WebUI),
                ..BuildOptions::default()
            })
            .unwrap();
            assert!(result.protocol.tokens.is_empty(), "{dom:?} {css:?}");
            assert!(result.warnings.is_empty(), "{dom:?} {css:?}");
        }
    }
}

#[test]
fn scoped_defaults_do_not_escape_to_other_components_or_stylesheets() {
    let app = tempfile::tempdir().unwrap();
    for (name, content) in [
        (
            "index.html",
            "<local-token></local-token><other-token></other-token>",
        ),
        ("local-token.html", "<other-token></other-token>"),
        ("local-token.css", "local-token { --local: red; }"),
        ("other-token.html", "<span>Needs a theme value</span>"),
    ] {
        fs::write(app.path().join(name), content).unwrap();
    }
    for dom in [DomStrategy::Light, DomStrategy::Shadow] {
        for css in [
            "span { color: var(--local); }",
            "local-token span { color: var(--local); }",
        ] {
            fs::write(app.path().join("other-token.css"), css).unwrap();
            let error = build(BuildOptions {
                app_dir: app.path().to_path_buf(),
                dom,
                theme: Some(TokenFile {
                    themes: HashMap::from([("light".to_string(), HashMap::new())]),
                }),
                ..BuildOptions::default()
            })
            .unwrap_err();
            let webui::WebUIError::Parse {
                source: webui_parser::ParserError::Template(diagnostic),
                ..
            } = error
            else {
                panic!("expected a structured missing-theme-token diagnostic");
            };
            assert_eq!(
                diagnostic.error_code(),
                Some(webui_parser::codes::MISSING_THEME_TOKEN)
            );
            assert_eq!(diagnostic.component_name(), Some("other-token.css"));
        }
    }
}

#[test]
fn inline_styles_share_the_same_selector_inheritance_analysis() {
    let app = tempfile::tempdir().unwrap();
    fs::write(
            app.path().join("index.html"),
            "<style>.scope { --local: red; } .scope span { color: var(--local); }</style><div class=\"scope\"><span>Local</span></div>",
        ).unwrap();
    let result = build(BuildOptions {
        app_dir: app.path().to_path_buf(),
        theme: Some(TokenFile {
            themes: HashMap::from([("light".to_string(), HashMap::new())]),
        }),
        ..BuildOptions::default()
    })
    .unwrap();
    assert!(result.protocol.tokens.is_empty());
}
