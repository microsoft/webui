// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! `buildApp`: protocol, lazy-region packs, and the generated `lazy.webui.js`
//! with its `lazy.webui.d.ts`.

use super::{compile, component_script, parse_dom};
use crate::error::WasmError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use webui_handler::lazy_packs::{
    emit, module_specifier, render_scripts_module, startup_components, LazyPackFile, LazyScript,
    LAZY_SCRIPTS_DECLARATION, LAZY_SCRIPTS_DECLARATION_SOURCE, LAZY_SCRIPTS_MODULE,
};
use webui_parser::component_modules;
use webui_protocol::projection_manifest::ProjectionManifest;

/// Options accepted by `buildApp`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AppOptions {
    /// Projection manifest objects, merged in order.
    #[serde(default)]
    pub(crate) projection_manifests: Vec<ProjectionManifest>,
    /// `"shadow"` (default) or `"light"` for unwrapped components.
    pub(crate) dom: Option<String>,
    /// Public URL or path ending in `/` that maps to the output directory.
    pub(crate) asset_base: Option<String>,
    /// Module specifiers keyed by custom element tag.
    #[serde(default)]
    pub(crate) component_modules: BTreeMap<String, String>,
    /// Virtual directory that will receive `lazy.webui.js`; the file map root
    /// by default.
    pub(crate) modules_out: Option<String>,
}

/// A `buildApp` result before conversion to JavaScript values.
pub(crate) struct App {
    pub(crate) protocol: Vec<u8>,
    pub(crate) packs: Vec<LazyPackFile>,
    pub(crate) lazy_scripts_module: Option<String>,
    pub(crate) warnings: Vec<String>,
}

/// Build protocol bytes, lazy-region template packs, and `lazy.webui.js`.
///
/// `options` is an optional object with `projectionManifests`, `dom`
/// (`"shadow"` or `"light"`), `assetBase` (a public URL or path ending in `/`
/// that maps to the output directory; requests override it with
/// `$webui.assetBase`), `componentModules` (module specifiers keyed by custom
/// element tag), and `modulesOut` (the virtual directory that will receive
/// `lazy.webui.js`; relative specifiers start there, at the file map root by
/// default).
///
/// Returns `{ protocol, lazyPackFiles, lazyScriptsModule?, warnings }`.
/// `protocol` is a `Uint8Array`. Each `lazyPackFiles` item has `fileName`,
/// `content`, `path`, `region`, `owner`, `trigger`, `hash`, `bytes`,
/// `scripts`, and `stylesheets`; publish its content at `path` in the output
/// directory. WASM builds inline component CSS, so `stylesheets` is always
/// empty, and they do not measure brotli sizes. `lazyScriptsModule` is
/// `{ fileName, content, declaration: { fileName, content } }`, the generated
/// `lazy.webui.js` and its `lazy.webui.d.ts`, when the build has lazy regions.
#[wasm_bindgen(js_name = buildApp)]
pub fn build_app(
    files: JsValue,
    entry: &str,
    options: Option<JsValue>,
) -> Result<JsValue, JsValue> {
    let files: HashMap<String, String> = serde_wasm_bindgen::from_value(files)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    let options: AppOptions = match options {
        Some(value) if !value.is_undefined() && !value.is_null() => {
            reject_unknown_options(&value)?;
            serde_wasm_bindgen::from_value(value)
                .map_err(|error| JsValue::from_str(&format!("invalid buildApp options: {error}")))?
        }
        _ => AppOptions::default(),
    };
    let app = build_app_inner(&files, entry, &options)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    to_js(&app)
}

/// Option names `buildApp` accepts. Deserializing a struct from JavaScript
/// reads only known properties, so misspelled options are rejected here.
const OPTION_NAMES: [&str; 5] = [
    "projectionManifests",
    "dom",
    "assetBase",
    "componentModules",
    "modulesOut",
];

fn reject_unknown_options(value: &JsValue) -> Result<(), JsValue> {
    let Some(object) = value.dyn_ref::<js_sys::Object>() else {
        return Ok(());
    };
    for key in js_sys::Object::keys(object).iter() {
        let key = key.as_string().unwrap_or_default();
        if !OPTION_NAMES.contains(&key.as_str()) {
            return Err(JsValue::from_str(&format!(
                "invalid buildApp options: unknown option `{key}`; expected {}",
                OPTION_NAMES.join(", ")
            )));
        }
    }
    Ok(())
}

pub(crate) fn build_app_inner(
    files: &HashMap<String, String>,
    entry: &str,
    options: &AppOptions,
) -> Result<App, WasmError> {
    let asset_base = options.asset_base.clone().unwrap_or_default();
    if !asset_base.is_empty() {
        webui_protocol::validate_asset_base(&asset_base)
            .map_err(|error| WasmError::InvalidBuild(error.to_string()))?;
    }
    for (tag, specifier) in &options.component_modules {
        component_modules::validate_component_module(tag, specifier)
            .map_err(WasmError::ComponentModules)?;
    }
    let dom = parse_dom(options.dom.clone()).map_err(WasmError::InvalidBuild)?;
    let (mut protocol, diagnostics) =
        compile(files, entry, &options.projection_manifests, dom, asset_base)?;
    let mut warnings: Vec<String> = diagnostics.iter().map(ToString::to_string).collect();
    unknown_mapping_warnings(files, entry, &options.component_modules, &mut warnings);
    // Packs are referenced from final metadata, so identity is assigned last.
    let mut packs = emit(&mut protocol).map_err(|error| WasmError::LazyPacks(error.to_string()))?;
    protocol.assign_build_id().map_err(WasmError::Protocol)?;
    let modules_dir = Path::new(options.modules_out.as_deref().unwrap_or_default());
    let startup = if packs.is_empty() {
        HashSet::new()
    } else {
        startup_components(&protocol, entry)
    };
    attach_scripts(
        &mut packs,
        files,
        &options.component_modules,
        modules_dir,
        &startup,
    )?;
    Ok(App {
        protocol: protocol.to_protobuf().map_err(WasmError::Protocol)?,
        lazy_scripts_module: render_scripts_module(&packs),
        packs,
        warnings,
    })
}

/// Warn about mappings whose tags no virtual component file declares.
fn unknown_mapping_warnings(
    files: &HashMap<String, String>,
    entry: &str,
    mappings: &BTreeMap<String, String>,
    warnings: &mut Vec<String>,
) {
    let mut known: Option<Vec<&str>> = None;
    for (tag, specifier) in mappings {
        let template = format!("{tag}.html");
        if template != entry && files.contains_key(&template) {
            continue;
        }
        let names = known.get_or_insert_with(|| {
            let mut names: Vec<&str> = files
                .keys()
                .filter(|name| name.as_str() != entry)
                .filter_map(|name| name.strip_suffix(".html"))
                .filter(|tag| tag.contains('-'))
                .collect();
            names.sort_unstable();
            names
        });
        warnings.push(
            component_modules::unknown_component_module(tag, specifier, names.iter().copied())
                .to_string(),
        );
    }
}

/// Record each pack's scripts: a mapping, else the component's virtual module
/// relative to `modules_dir`. Components in `startup`, which the page loads
/// before any region opens, are never region scripts.
fn attach_scripts(
    packs: &mut [LazyPackFile],
    files: &HashMap<String, String>,
    mappings: &BTreeMap<String, String>,
    modules_dir: &Path,
    startup: &HashSet<&str>,
) -> Result<(), WasmError> {
    for pack in packs {
        let mut scripts = Vec::new();
        for tag in &pack.components {
            if startup.contains(tag.as_str()) {
                continue;
            }
            let specifier = if let Some(specifier) = mappings.get(tag) {
                specifier.clone()
            } else if let Some(module) = component_script(files, tag) {
                module_specifier(modules_dir, Path::new(module))
                    .ok_or_else(|| unreachable_module(tag, module, modules_dir))?
            } else {
                continue;
            };
            scripts.push(LazyScript {
                tag: tag.clone(),
                specifier,
            });
        }
        pack.scripts = scripts;
    }
    Ok(())
}

#[cold]
#[inline(never)]
fn unreachable_module(tag: &str, module: &str, modules_dir: &Path) -> WasmError {
    WasmError::ComponentModules(Box::new(component_modules::unreachable_component_module(
        tag,
        Path::new(module),
        modules_dir,
    )))
}

/// One lazy pack as a JavaScript object.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsPack<'a> {
    file_name: &'a str,
    content: &'a str,
    path: String,
    region: &'a str,
    owner: &'a str,
    trigger: &'static str,
    hash: &'a str,
    bytes: usize,
    scripts: Vec<&'a str>,
    stylesheets: &'a [String],
}

/// The generated module and its declarations as a JavaScript object.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsModule<'a> {
    file_name: &'static str,
    content: &'a str,
    declaration: JsDeclaration,
}

/// `lazy.webui.d.ts` as a JavaScript object.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsDeclaration {
    file_name: &'static str,
    content: &'static str,
}

fn to_js(app: &App) -> Result<JsValue, JsValue> {
    let packs: Vec<JsPack<'_>> = app
        .packs
        .iter()
        .map(|pack| JsPack {
            file_name: &pack.file_name,
            content: &pack.content,
            path: pack.path(),
            region: &pack.region,
            owner: &pack.owner,
            trigger: pack.trigger.as_str(),
            hash: pack.hash(),
            bytes: pack.bytes(),
            scripts: pack
                .scripts
                .iter()
                .map(|script| script.specifier.as_str())
                .collect(),
            stylesheets: &pack.stylesheets,
        })
        .collect();
    let result = js_sys::Object::new();
    js_sys::Reflect::set(
        &result,
        &"protocol".into(),
        &js_sys::Uint8Array::from(app.protocol.as_slice()),
    )?;
    js_sys::Reflect::set(&result, &"lazyPackFiles".into(), &serialize(&packs)?)?;
    if let Some(content) = &app.lazy_scripts_module {
        let module = JsModule {
            file_name: LAZY_SCRIPTS_MODULE,
            content,
            declaration: JsDeclaration {
                file_name: LAZY_SCRIPTS_DECLARATION,
                content: LAZY_SCRIPTS_DECLARATION_SOURCE,
            },
        };
        js_sys::Reflect::set(&result, &"lazyScriptsModule".into(), &serialize(&module)?)?;
    }
    js_sys::Reflect::set(&result, &"warnings".into(), &serialize(&app.warnings)?)?;
    Ok(result.into())
}

fn serialize(value: &impl Serialize) -> Result<JsValue, JsValue> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use webui_handler::lazy_packs::LazyTrigger;
    use webui_protocol::WebUIProtocol;

    fn files(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries
            .iter()
            .map(|(name, content)| ((*name).to_string(), (*content).to_string()))
            .collect()
    }

    fn lazy_files() -> HashMap<String, String> {
        files(&[
            (
                "index.html",
                "<html><body><app-owner></app-owner></body></html>",
            ),
            (
                "app-owner.html",
                r#"<if condition="show" lazy><settings-panel></settings-panel><if condition="deep" lazy="demand"><theme-picker></theme-picker></if></if>"#,
            ),
            ("settings-panel.html", "<p>Settings</p>"),
            ("settings-panel.ts", "export {};"),
            ("settings-panel.js", "export {};"),
            ("theme-picker.html", "<select></select>"),
            ("theme-picker.js", "export {};"),
        ])
    }

    #[test]
    fn lazy_regions_emit_packs_with_scripts_and_a_loader_module() {
        let options = AppOptions {
            asset_base: Some("https://cdn.example.com/app/".to_string()),
            ..AppOptions::default()
        };
        let mut files = lazy_files();
        files.insert(
            "settings-panel.css".to_string(),
            "p { margin: 0; }".to_string(),
        );
        let app = build_app_inner(&files, "index.html", &options).unwrap();
        assert_eq!(app.packs.len(), 2);
        let outer = app
            .packs
            .iter()
            .find(|pack| pack.region == "app-owner::lazy:@0")
            .unwrap();
        assert!(
            outer.file_name.starts_with("app-owner."),
            "{}",
            outer.file_name
        );
        assert!(outer.content.starts_with(r#"{"type":"webui-pack""#));
        assert!(outer.content.contains("p { margin: 0; }"));
        assert_eq!(outer.trigger, LazyTrigger::Intent);
        assert_eq!(
            outer.brotli_bytes, None,
            "WASM builds do not measure brotli"
        );
        assert!(
            app.packs.iter().all(|pack| pack.stylesheets.is_empty()),
            "WASM builds inline component CSS"
        );
        assert_eq!(
            outer.scripts,
            [LazyScript {
                tag: "settings-panel".to_string(),
                specifier: "./settings-panel.ts".to_string(),
            }]
        );
        let module = app.lazy_scripts_module.unwrap();
        assert!(module.contains(r#""settings-panel": () => import("./settings-panel.ts"),"#));
        assert!(module.contains(r#""theme-picker": () => import("./theme-picker.js"),"#));

        let protocol = WebUIProtocol::from_protobuf(&app.protocol).unwrap();
        // Pack paths stay relative to the asset base the protocol stores.
        let reference = format!(
            r#""lp":{{"app-owner::lazy:@0":"_webui/packs/{}"}}"#,
            outer.file_name
        );
        assert!(protocol.components["app-owner"]
            .template_json
            .contains(&reference));
        assert_eq!(protocol.asset_base, "https://cdn.example.com/app/");
        assert!(protocol.build_id.starts_with("sha256:"));
    }

    #[test]
    fn modules_out_and_mappings_shape_specifiers() {
        let options = AppOptions {
            modules_out: Some(".webui".to_string()),
            component_modules: BTreeMap::from([
                (
                    "theme-picker".to_string(),
                    "@contoso/themes/picker.js".to_string(),
                ),
                ("unused-card".to_string(), "./unused.js".to_string()),
            ]),
            ..AppOptions::default()
        };
        let app = build_app_inner(&lazy_files(), "index.html", &options).unwrap();
        let module = app.lazy_scripts_module.unwrap();
        assert!(module.contains(r#""settings-panel": () => import("../settings-panel.ts"),"#));
        assert!(module.contains(r#""theme-picker": () => import("@contoso/themes/picker.js"),"#));
        assert!(app
            .warnings
            .iter()
            .any(|warning| warning.contains("unknown-component-module")));

        let invalid = AppOptions {
            component_modules: BTreeMap::from([(
                "settings-panel".to_string(),
                "./has space.js".to_string(),
            )]),
            ..AppOptions::default()
        };
        let Err(error) = build_app_inner(&lazy_files(), "index.html", &invalid) else {
            panic!("invalid mapping must fail");
        };
        assert!(
            error.to_string().contains("invalid-component-module"),
            "{error}"
        );

        let escaping = AppOptions {
            modules_out: Some("../out".to_string()),
            ..AppOptions::default()
        };
        let Err(error) = build_app_inner(&lazy_files(), "index.html", &escaping) else {
            panic!("a modules directory above the file map root has no relative path");
        };
        assert!(
            error.to_string().contains("unreachable-component-module"),
            "{error}"
        );
    }

    #[test]
    fn builds_without_lazy_regions_have_no_loader_module() {
        let app = build_app_inner(
            &files(&[("index.html", "<p>Static</p>")]),
            "index.html",
            &AppOptions::default(),
        )
        .unwrap();
        assert!(app.packs.is_empty());
        assert!(app.lazy_scripts_module.is_none());
    }

    #[test]
    fn components_the_page_loads_at_startup_are_not_region_scripts() {
        let mut files = lazy_files();
        for (name, content) in [
            (
                "index.html",
                "<html><body><user-badge></user-badge><app-owner></app-owner></body></html>",
            ),
            (
                "app-owner.html",
                r#"<if condition="show" lazy><user-badge></user-badge><settings-panel></settings-panel></if>"#,
            ),
            ("user-badge.html", "<span>User</span>"),
            ("user-badge.ts", "export {};"),
        ] {
            files.insert(name.to_string(), content.to_string());
        }
        let app = build_app_inner(&files, "index.html", &AppOptions::default()).unwrap();
        let [pack] = app.packs.as_slice() else {
            panic!("expected one pack, got {:?}", app.packs);
        };
        assert!(
            pack.components.iter().any(|tag| tag == "user-badge"),
            "the pack still carries the template"
        );
        let tags: Vec<&str> = pack
            .scripts
            .iter()
            .map(|script| script.tag.as_str())
            .collect();
        assert_eq!(tags, ["settings-panel"]);
        assert!(!app.lazy_scripts_module.unwrap().contains("user-badge"));
    }
}
