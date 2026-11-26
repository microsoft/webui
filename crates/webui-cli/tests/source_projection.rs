// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use anyhow::{Context, Result};
use serde_json::{json, Value};
use webui::{Protocol, RenderOptions, WebUIHandler, WebUIProtocol};
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_protocol::{InitialStateStrategy, StateProjectionMode};

webui_handler::define_string_response_writer!(Html, output);

#[test]
fn plain_webui_build_projects_the_active_route_without_touching_browser_code() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let src = temp.path().join("src");
    let dist = temp.path().join("dist");
    fs::create_dir_all(src.join("cards"))?;
    fs::create_dir(&dist)?;
    let bundled = dist.join("index.js");
    let bundle_bytes = "import './chunks/client.js';";
    fs::write(&bundled, bundle_bytes)?;
    let modified = fs::metadata(&bundled)?.modified()?;
    for (file, content) in [
        (
            "index.html",
            r#"<html><head></head><body>
          <route path="/" component="route-root">
            <route path="" component="exact-card" exact />
            <route path="empty" component="empty-card" exact />
            <route path="unknown" component="unknown-card" exact />
          </route><script type="module" src="/index.js"></script>
        </body></html>"#,
        ),
        ("route-root.html", "<outlet/>"),
        (
            "base.ts",
            r#"
          import { WebUIElement as Element, observable as state } from '@microsoft/webui-framework';
          import { recordMember } from './metadata.js';
          export class Base extends Element { @recordMember @state count = 0; }
        "#,
        ),
        (
            "metadata.ts",
            r#"
          const lastMember = new WeakMap<object, string>();
          export function recordMember(target: object, key: string): void {
            lastMember.set(target, key);
          }
          export function memberName(target: object) { return lastMember.get(target); }
          export function describe(label: string) {
            return (target: object, _key: string, descriptor: PropertyDescriptor) => {
              lastMember.set(target, label);
              return descriptor;
            };
          }
        "#,
        ),
        (
            "mixin.ts",
            r#"
          import { observable } from '@microsoft/webui-framework';
          import { describe, recordMember } from './metadata.js';
          export function withStatus(Base) {
            return class extends Base {
              @observable status = '';
              @describe('retry') @recordMember retry() {}
            };
          }
        "#,
        ),
        ("cards/exact-card.html", "<p>{{title}}</p>"),
        (
            "cards/exact-card.ts",
            r#"
          import * as web from '@microsoft/webui-framework';
          import { Base as Inherited } from '../base.js';
          import { withStatus } from '../mixin.js';
          class Card extends withStatus(Inherited) {
            @web.attr({ attribute: 'data-caption' }) caption = '';
          }
          const Definition = Card;
          Definition.define('exact-card');
        "#,
        ),
        ("empty-card.html", "<p>{{emptyTitle}}</p>"),
        (
            "empty-card.ts",
            r#"
          import { WebUIElement } from '@microsoft/webui-framework';
          import { recordMember } from './metadata.js';
          class Empty extends WebUIElement { @recordMember internal = ''; }
          Empty.define('empty-card');
        "#,
        ),
        ("unknown-card.html", "<p>{{title}}</p>"),
        (
            "unknown-card.ts",
            r#"
          import { WebUIElement, observable } from '@microsoft/webui-framework';
          function reactive(target: object, key: string) { observable(target, key); }
          function withUnknown(Base) {
            return class extends Base {
              @reactive hidden = '';
              @observable count = 0;
            };
          }
          class Unknown extends withUnknown(WebUIElement) {}
          Unknown.define('unknown-card');
        "#,
        ),
    ] {
        fs::write(src.join(file), content)?;
    }
    let output = cli(
        temp.path(),
        &[
            "--format",
            "json",
            "build",
            "src",
            "--plugin=webui",
            "--out",
            "dist",
        ],
    )?;
    let warning: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(warning["severity"], "warning");
    assert_eq!(warning["code"], "PROJ-B003");
    assert_eq!(warning["file"], "unknown-card");
    assert!(warning["help"]
        .as_str()
        .is_some_and(|help| help.contains("decorators")));
    assert_eq!(fs::read_to_string(&bundled)?, bundle_bytes);
    assert_eq!(fs::metadata(&bundled)?.modified()?, modified);
    assert_eq!(fs::read_dir(&dist)?.count(), 2, "only write protocol.bin");

    let bytes = fs::read(dist.join("protocol.bin"))?;
    let compiled = WebUIProtocol::from_protobuf(&bytes)?;
    assert_eq!(
        compiled.initial_state_strategy(),
        InitialStateStrategy::Components
    );
    assert!(compiled.module_preloads.is_empty());
    assert_eq!(
        compiled.components["empty-card"].hydration_mode(),
        StateProjectionMode::None
    );
    assert_eq!(
        compiled.components["empty-card"].navigation_keys,
        ["emptyTitle"]
    );
    assert_eq!(
        compiled.components["unknown-card"].hydration_mode(),
        StateProjectionMode::All
    );
    assert_eq!(
        compiled.components["unknown-card"].navigation_mode(),
        StateProjectionMode::All
    );
    let protocol = Protocol::new(compiled);
    let state = json!({
        "caption": "Visible", "count": 7, "status": "Ready", "title": "Rendered title",
        "emptyTitle": "Unused route", "hidden": "runtime decorator", "unrelated": [1, 2, 3]
    });
    assert_eq!(
        bootstrap(&protocol, "/", &state)?,
        json!({"caption":"Visible","count":7,"status":"Ready"})
    );
    assert_eq!(bootstrap(&protocol, "/empty", &state)?, json!({}));
    assert_eq!(bootstrap(&protocol, "/unknown", &state)?, state);
    let partial = |path| -> Result<Value> {
        let json = protocol.render_partial_json(&state.to_string(), "index.html", path, "")?;
        Ok(serde_json::from_str::<Value>(&json)?["state"].take())
    };
    assert_eq!(
        partial("/")?,
        json!({"caption":"Visible","count":7,"status":"Ready","title":"Rendered title"})
    );
    assert_eq!(partial("/empty")?, json!({"emptyTitle":"Unused route"}));
    assert_eq!(partial("/unknown")?, state);
    Ok(())
}

#[test]
fn ambiguous_decorators_keep_full_state_even_with_framework_fields() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let diagnostics = [
        (
            "wrapped-field",
            "function extra(target: object) { observable(target, 'extra'); }",
            "@extra @observable known = '';",
        ),
        (
            "wrapped-method",
            "function extra(target: object) { observable(target, 'extra'); }",
            "@extra connectedCallback() { super.connectedCallback(); }",
        ),
        (
            "exported-map",
            "const metadata = new WeakMap<object, string>(); export { metadata };
             function record(target: object, key: string) { metadata.set(target, key); }",
            "@record @observable known = '';",
        ),
        (
            "overridden-map",
            "const metadata = new WeakMap<object, string>();
             metadata.set = function (target, key) { observable(target, key); return this; };
             function record(target: object, key: string) { metadata.set(target, key); }",
            "@record @observable known = '';",
        ),
        (
            "evaluated-map",
            "const metadata = new WeakMap<object, string>();
             function readCache() { return eval('metadata'); }
             function record(target: object, key: string) { metadata.set(target, key); }",
            "@record @observable known = '';",
        ),
        (
            "dynamic-options",
            "function decorate(_value: string) { return () => {}; }
             function option() { observable(Card.prototype, 'extra'); return ''; }",
            "@decorate(option()) @observable known = '';",
        ),
    ];
    let mut html = String::new();
    for (tag, helpers, member) in diagnostics {
        html.push_str(&format!("<{tag}></{tag}>"));
        fs::write(root.join(format!("{tag}.html")), "<p>{{title}}</p>")?;
        fs::write(
            root.join(format!("{tag}.ts")),
            format!(
                "import {{ WebUIElement, observable }} from '@microsoft/webui-framework';
                 {helpers}
                 class Card extends WebUIElement {{ @observable safe = ''; {member} }}
                 Card.define('{tag}');"
            ),
        )?;
    }
    fs::write(root.join("index.html"), html)?;
    let output = cli(
        root,
        &[
            "--format",
            "json",
            "build",
            "--plugin=webui",
            "--out",
            "dist",
        ],
    )?;
    let mut warned = std::collections::BTreeSet::new();
    let stdout = String::from_utf8(output.stdout)?;
    for line in stdout.lines() {
        let warning: Value = serde_json::from_str(line)?;
        assert_eq!(warning["code"], "PROJ-B003");
        warned.insert(
            warning["file"]
                .as_str()
                .context("warning owner")?
                .to_owned(),
        );
    }
    let protocol = WebUIProtocol::from_protobuf(&fs::read(root.join("dist/protocol.bin"))?)?;
    for (tag, _, _) in diagnostics {
        assert!(warned.remove(tag), "{tag} must warn");
        let component = &protocol.components[tag];
        assert_eq!(
            component.hydration_mode(),
            StateProjectionMode::All,
            "{tag}"
        );
        assert_eq!(
            component.navigation_mode(),
            StateProjectionMode::All,
            "{tag}"
        );
        assert!(component.hydration_keys.is_empty(), "{tag}");
    }
    assert!(warned.is_empty(), "unexpected warnings: {warned:?}");
    Ok(())
}

#[test]
fn managed_bundle_proof_still_supplies_static_preloads() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    fs::create_dir(root.join("src"))?;
    for (path, content) in [
        (
            "src/index.html",
            r#"<html><head></head><body><script type="module" src="/assets/index.js"></script></body></html>"#,
        ),
        (
            "src/entry.ts",
            "import { shared } from './shared.ts'; console.log(shared);",
        ),
        (
            "src/other.ts",
            "import { shared } from './shared.ts'; console.info(shared);",
        ),
        ("src/shared.ts", "export const shared = Math.random();"),
        (
            "browser.json",
            r#"{"project":".","entries":{"index":"src/entry.ts","other":"src/other.ts"},"outDir":"dist"}"#,
        ),
    ] {
        fs::write(root.join(path), content)?;
    }
    cli(root, &["bundle", "--config", "browser.json"])?;
    cli(root, &["build", "src", "--plugin=webui", "--out", "dist"])?;
    let read_protocol = || -> Result<WebUIProtocol> {
        Ok(WebUIProtocol::from_protobuf(&fs::read(
            root.join("dist/protocol.bin"),
        )?)?)
    };
    assert!(read_protocol()?.module_preloads.is_empty());
    cli(
        root,
        &[
            "build",
            "src",
            "--plugin=webui",
            "--out",
            "dist",
            "--projection-manifest",
            "dist/webui-projection.json",
        ],
    )?;
    let preloads = read_protocol()?.module_preloads;
    assert_eq!(preloads.len(), 1, "{preloads:?}");
    let file = preloads[0]
        .strip_prefix("/assets/")
        .context("served asset path")?;
    assert!(root.join("dist").join(file).is_file());
    Ok(())
}

fn cli(dir: &Path, args: &[&str]) -> Result<Output> {
    let output = Command::new(env!("CARGO_BIN_EXE_webui"))
        .current_dir(dir)
        .args(args)
        .output()?;
    assert!(
        output.status.success(),
        "webui {args:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    Ok(output)
}

fn bootstrap(protocol: &Protocol, path: &str, state: &Value) -> Result<Value> {
    let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
    let mut html = Html::with_capacity(1024);
    handler.render(
        protocol,
        state,
        &RenderOptions::new("index.html", path),
        &mut html,
    )?;
    let (_, data) = html
        .output
        .split_once("id=\"webui-data\"")
        .context("bootstrap script")?;
    let (_, json) = data.split_once('>').context("bootstrap payload")?;
    let (json, _) = json.split_once("</script>").context("bootstrap end")?;
    Ok(serde_json::from_str::<Value>(json)?["state"].take())
}
