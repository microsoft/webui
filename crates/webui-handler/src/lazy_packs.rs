// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Content-addressed template packs for lazy regions.
//!
//! Each lazy region becomes one immutable JSON file. A template containing a
//! lazy condition carries an `lp` map from region identity to pack path, so a
//! pack that serializes such a template depends on the paths of the packs it
//! references; prefetch triggers stay in the condition entries. In Link builds
//! the same template carries an `lc` map from region identity to the
//! stylesheet paths the region's pack registers, so the browser can fetch them
//! in parallel with the pack. Packs are rendered in dependency passes: a
//! region renders once every region referenced by its templates has a path.
//! Build identity is assigned afterwards, over final metadata.
//!
//! Pack paths are `_webui/packs/<file>`, relative to the asset base, which maps
//! to the build output directory. The browser resolves them against the base it
//! receives with the document.
//!
//! Packs carry templates and styles, never JavaScript. Custom elements that a
//! region introduces load through the application's bundler instead: builds
//! record each region's component modules in [`LazyPackFile::scripts`],
//! leaving out the [`startup_components`] the page already loads, and
//! [`render_scripts_module`] writes them as dynamic imports in
//! [`LAZY_SCRIPTS_MODULE`].

mod references;
mod scripts;
mod startup;

use crate::{HandlerError, Protocol, Result};
use references::{collect_references, encode_stylesheets, finalize, References, Rendered};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use webui_protocol::WebUIProtocol;

pub use scripts::{
    module_specifier, render_scripts_module, LAZY_SCRIPTS_DECLARATION,
    LAZY_SCRIPTS_DECLARATION_SOURCE, LAZY_SCRIPTS_MODULE,
};
pub use startup::startup_components;

/// Output directory for pack files, relative to the build output directory.
///
/// Pack paths in template metadata are this directory plus the file name.
pub const PACK_OUTPUT_DIR: &str = "_webui/packs";

/// When a lazy region's pack may be fetched before its condition is true.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LazyTrigger {
    /// Pointer, focus, or touch intent inside the owner (`lazy`).
    Intent,
    /// The first idle period after the owner mounts (`lazy="idle"`).
    Idle,
    /// Right after the owner hydrates or mounts (`lazy="eager"`).
    Eager,
    /// Only when the condition becomes true (`lazy="demand"`).
    Demand,
}

impl LazyTrigger {
    /// The authored trigger name: `intent`, `idle`, `eager`, or `demand`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Intent => "intent",
            Self::Idle => "idle",
            Self::Eager => "eager",
            Self::Demand => "demand",
        }
    }

    /// Decode the trigger code of a compiled condition entry.
    const fn from_code(code: u64) -> Option<Self> {
        match code {
            1 => Some(Self::Intent),
            2 => Some(Self::Idle),
            3 => Some(Self::Eager),
            4 => Some(Self::Demand),
            _ => None,
        }
    }
}

/// The module that defines a custom element a lazy region introduces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LazyScript {
    /// Custom element tag the module defines.
    pub tag: String,
    /// Module specifier, written verbatim as a dynamic import.
    pub specifier: String,
}

/// One immutable lazy-region pack and its cost report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LazyPackFile {
    /// Content-addressed file name inside [`PACK_OUTPUT_DIR`].
    pub file_name: String,
    /// JSON pack content. Its length is the pack's raw size in bytes.
    pub content: String,
    /// Region identity, `<owner>::lazy:@<n>`.
    pub region: String,
    /// Component whose template declares the region.
    pub owner: String,
    /// When the pack may be fetched before the region opens.
    pub trigger: LazyTrigger,
    /// Component tags whose templates the pack carries, sorted: the
    /// components the region introduces beyond those its owner guarantees.
    pub components: Vec<String>,
    /// Modules of the scripted components the region introduces, sorted by
    /// tag, except the [`startup_components`] the page already loads.
    ///
    /// [`emit`] leaves this empty. Builds fill it from discovered component
    /// modules and explicit module mappings.
    pub scripts: Vec<LazyScript>,
    /// Brotli size of `content` at quality 11, when the build computed a cost
    /// report. [`emit`] leaves this `None`.
    pub brotli_bytes: Option<usize>,
}

impl LazyPackFile {
    /// The pack's path relative to the asset base: `_webui/packs/<file>`.
    #[must_use]
    pub fn path(&self) -> String {
        pack_path(&self.file_name)
    }

    /// The pack's raw size in bytes.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.content.len()
    }

    /// The content hash in the file name: the first 16 hex digits of the
    /// SHA-256 of `content`.
    #[must_use]
    pub fn hash(&self) -> &str {
        self.file_name
            .strip_suffix(".json")
            .and_then(|stem| stem.rsplit_once('.'))
            .map_or("", |(_, hash)| hash)
    }
}

/// Render every lazy-region pack and reference it from final template metadata.
///
/// Call after compiling the protocol and before assigning its build identity.
///
/// # Errors
///
/// Returns [`HandlerError::Rendering`] when lazy regions reference each other
/// in a cycle and [`HandlerError::Invariant`] for malformed compiled metadata.
pub fn emit(protocol: &mut WebUIProtocol) -> Result<Vec<LazyPackFile>> {
    let mut regions: Vec<String> = protocol
        .client_fragments
        .iter()
        .filter(|(_, fragment)| fragment.entry)
        .map(|(id, _)| id.clone())
        .collect();
    if regions.is_empty() {
        return Ok(Vec::new());
    }
    regions.sort_unstable();
    let References { children, triggers } = collect_references(protocol)?;
    let initial = Protocol::new(protocol.clone());
    let mut contents = HashMap::with_capacity(regions.len());
    for region in &regions {
        contents.insert(region.as_str(), initial.lazy_pack_templates(region)?);
    }
    drop(initial);

    let mut rendered: HashMap<String, Rendered> = HashMap::with_capacity(regions.len());
    let mut finalized = HashSet::new();
    let mut files = Vec::with_capacity(regions.len());
    let mut pending: Vec<&str> = regions.iter().map(String::as_str).collect();
    while !pending.is_empty() {
        let (ready, blocked): (Vec<&str>, Vec<&str>) = std::mem::take(&mut pending)
            .into_iter()
            .partition(|region| {
                contents[region].iter().all(|template| {
                    children
                        .get(template)
                        .is_none_or(|refs| refs.iter().all(|child| rendered.contains_key(child)))
                })
            });
        if ready.is_empty() {
            return Err(cyclic_packs(&blocked));
        }
        finalize(protocol, &children, &rendered, &mut finalized)?;
        let view = Protocol::new(protocol.clone());
        for region in ready {
            let (content, stylesheets) = view.render_lazy_pack_with_stylesheets(region)?;
            let owner = protocol.client_fragments[region].owner.clone();
            let file_name = file_name(&owner, &content);
            let trigger = triggers
                .get(region)
                .copied()
                .ok_or_else(|| invalid_metadata(region, "no template declares this region"))?;
            let components = contents[region]
                .iter()
                .filter(|id| protocol.components.contains_key(id.as_str()))
                .cloned()
                .collect();
            rendered.insert(
                region.to_owned(),
                Rendered {
                    path: pack_path(&file_name),
                    stylesheets: encode_stylesheets(&stylesheets),
                },
            );
            files.push(LazyPackFile {
                file_name,
                content,
                region: region.to_owned(),
                owner,
                trigger,
                components,
                scripts: Vec::new(),
                brotli_bytes: None,
            });
        }
        pending = blocked;
    }
    finalize(protocol, &children, &rendered, &mut finalized)?;
    files.sort_unstable_by(|left, right| left.file_name.cmp(&right.file_name));
    Ok(files)
}

/// `_webui/packs/<file>`: the pack's path relative to the asset base.
fn pack_path(file_name: &str) -> String {
    let mut path = String::with_capacity(PACK_OUTPUT_DIR.len() + 1 + file_name.len());
    path.push_str(PACK_OUTPUT_DIR);
    path.push('/');
    path.push_str(file_name);
    path
}

/// `<owner>.<first 16 hex digits of SHA-256>.json`, safe on every filesystem.
fn file_name(owner: &str, content: &str) -> String {
    let digest = Sha256::digest(content.as_bytes());
    let mut name = String::with_capacity(owner.len() + 22);
    for ch in owner.chars() {
        name.push(
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | '_') {
                ch
            } else {
                '_'
            },
        );
    }
    name.push('.');
    for byte in &digest[..8] {
        let _ = write!(name, "{byte:02x}");
    }
    name.push_str(".json");
    name
}

#[cold]
#[inline(never)]
fn cyclic_packs(regions: &[&str]) -> HandlerError {
    HandlerError::Rendering(format!(
        "lazy regions reference each other in a cycle: {}. Move the shared UI into an ordinary component or remove lazy from one region.",
        regions.join(", ")
    ))
}

#[cold]
#[inline(never)]
fn invalid_metadata(template: &str, error: &str) -> HandlerError {
    HandlerError::Invariant(format!(
        "invalid lazy metadata in {template}: {error}. Rebuild with matching WebUI compiler packages."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use webui_protocol::{
        lazy_fragment_id, ClientFragmentData, ComponentData, ConditionExpr, CssStrategy,
        FragmentList, WebUIFragment,
    };

    #[test]
    fn trigger_codes_match_the_authored_names() {
        let decoded =
            [1, 2, 3, 4].map(|code| LazyTrigger::from_code(code).map(LazyTrigger::as_str));
        assert_eq!(
            decoded,
            [Some("intent"), Some("idle"), Some("eager"), Some("demand")]
        );
        assert_eq!(LazyTrigger::from_code(0), None);
        assert_eq!(LazyTrigger::from_code(5), None);
    }

    /// `app-shell` owns an eager `shared-badge`, an intent region that adds
    /// `settings-form`, and a nested demand region inside it.
    fn fixture() -> WebUIProtocol {
        let root = lazy_fragment_id("app-shell", 0);
        let nested = lazy_fragment_id("app-shell", 1);
        let mut document = WebUIProtocol::new(HashMap::from([
            (
                "app-shell".to_owned(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::component("shared-badge"),
                        WebUIFragment::lazy_if(ConditionExpr::identifier("open"), &root),
                    ],
                    ..Default::default()
                },
            ),
            (
                root.clone(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::component("shared-badge"),
                        WebUIFragment::component("settings-form"),
                        WebUIFragment::lazy_if(ConditionExpr::identifier("advanced"), &nested),
                    ],
                    ..Default::default()
                },
            ),
            (nested.clone(), FragmentList::default()),
            ("shared-badge".to_owned(), FragmentList::default()),
            ("settings-form".to_owned(), FragmentList::default()),
        ]));
        for (tag, json) in [
            (
                "app-shell",
                format!(r#"{{"h":"<p></p>","c":[[0,"{root}",[0,0,0],1]],"v":3}}"#),
            ),
            ("shared-badge", r#"{"h":"<p>Badge</p>","v":3}"#.to_owned()),
            ("settings-form", r#"{"h":"<form></form>","v":3}"#.to_owned()),
        ] {
            document.components.insert(
                tag.to_owned(),
                ComponentData {
                    template_json: json,
                    ..Default::default()
                },
            );
        }
        for (id, json) in [
            (
                root.clone(),
                format!(r#"{{"h":"<p></p>","c":[[0,"{nested}",[0,0,0],4]],"v":3}}"#),
            ),
            (nested, r#"{"h":"<p>Deep</p>","v":3}"#.to_owned()),
        ] {
            document.client_fragments.insert(
                id,
                ClientFragmentData {
                    owner: "app-shell".into(),
                    entry: true,
                    template_json: json,
                    ..Default::default()
                },
            );
        }
        document
    }

    #[test]
    fn packs_report_their_region_owner_trigger_and_components() {
        let mut protocol = fixture();
        let mut packs = emit(&mut protocol).unwrap();
        packs.sort_unstable_by(|left, right| left.region.cmp(&right.region));
        let [outer, nested] = packs.as_slice() else {
            panic!("expected two packs, got {packs:?}");
        };
        assert_eq!(outer.region, "app-shell::lazy:@0");
        assert_eq!(outer.owner, "app-shell");
        assert_eq!(outer.trigger, LazyTrigger::Intent);
        // The owner already mounts `shared-badge`, so only the form is new.
        assert_eq!(outer.components, ["settings-form"]);
        assert_eq!(nested.region, "app-shell::lazy:@1");
        assert_eq!(nested.trigger, LazyTrigger::Demand);
        assert!(nested.components.is_empty());
        for pack in &packs {
            assert!(pack.scripts.is_empty());
            assert_eq!(pack.brotli_bytes, None);
            assert_eq!(pack.path(), format!("_webui/packs/{}", pack.file_name));
            assert_eq!(pack.bytes(), pack.content.len());
            assert_eq!(pack.hash().len(), 16);
            assert!(pack.file_name.ends_with(&format!(".{}.json", pack.hash())));
        }
    }

    /// [`fixture`] with a nested region that adds `theme-picker`, compiled
    /// with `strategy` and a stylesheet on each of the `styled` components.
    fn styled_fixture(strategy: CssStrategy, styled: &[&str]) -> WebUIProtocol {
        let mut document = fixture();
        document.fragments.insert(
            lazy_fragment_id("app-shell", 1),
            FragmentList {
                fragments: vec![WebUIFragment::component("theme-picker")],
                ..Default::default()
            },
        );
        document
            .fragments
            .insert("theme-picker".to_owned(), FragmentList::default());
        document.components.insert(
            "theme-picker".to_owned(),
            ComponentData {
                template_json: r#"{"h":"<select></select>","v":3}"#.to_owned(),
                ..Default::default()
            },
        );
        document.set_css_strategy(strategy);
        for tag in styled {
            let component = document.components.get_mut(*tag).unwrap();
            // Shadow, the build default: each component is its own CSS tree,
            // installed at the hook the compiler places in its template.
            component.uses_shadow_dom = true;
            match strategy {
                CssStrategy::Link => component.css_href = format!("{tag}.css"),
                CssStrategy::Style | CssStrategy::Module => {
                    component.css = format!(".{tag} {{ color: red; }}");
                }
            }
            document.fragments.insert(
                (*tag).to_owned(),
                FragmentList {
                    fragments: vec![WebUIFragment::signal(
                        format!("{}shadow_styles:{tag}", crate::STRUCTURAL_SIGNAL_PREFIX),
                        true,
                    )],
                    ..Default::default()
                },
            );
        }
        document.populate_style_closures(&["app-shell"]);
        document
    }

    fn template_json<'a>(protocol: &'a WebUIProtocol, id: &str) -> &'a str {
        protocol.components.get(id).map_or_else(
            || protocol.client_fragments[id].template_json.as_str(),
            |component| component.template_json.as_str(),
        )
    }

    #[test]
    fn link_builds_list_the_stylesheets_each_pack_registers_beside_its_path() {
        let styled = ["shared-badge", "settings-form", "theme-picker"];
        let mut protocol = styled_fixture(CssStrategy::Link, &styled);
        let packs = emit(&mut protocol).unwrap();
        let root = lazy_fragment_id("app-shell", 0);
        let nested = lazy_fragment_id("app-shell", 1);
        let pack = |region: &str| packs.iter().find(|pack| pack.region == region).unwrap();

        // The owner already loads the badge, so only the form's stylesheet is
        // the outer region's own. The nested region is listed where its
        // condition lives: in the outer region's template.
        let owner = template_json(&protocol, "app-shell");
        let expected = format!(
            r#","lp":{{"{root}":"{}"}},"lc":{{"{root}":["settings-form.css"]}}}}"#,
            pack(&root).path()
        );
        assert!(owner.ends_with(&expected), "{owner}");
        let region = template_json(&protocol, &root);
        let expected = format!(
            r#","lp":{{"{nested}":"{}"}},"lc":{{"{nested}":["theme-picker.css"]}}}}"#,
            pack(&nested).path()
        );
        assert!(region.ends_with(&expected), "{region}");
        let owner: serde_json::Value = serde_json::from_str(owner).unwrap();
        assert_eq!(owner["lc"][&root], serde_json::json!(["settings-form.css"]));

        // The listed paths are the hrefs the pack itself registers.
        let outer: serde_json::Value = serde_json::from_str(&pack(&root).content).unwrap();
        let resources = &outer["componentStyles"]["resources"];
        assert_eq!(resources["settings-form"]["href"], "settings-form.css");
        assert!(resources.get("shared-badge").is_none(), "{resources}");

        let mut again = styled_fixture(CssStrategy::Link, &styled);
        assert_eq!(emit(&mut again).unwrap(), packs);
        assert_eq!(again.components, protocol.components);
        assert_eq!(again.client_fragments, protocol.client_fragments);
    }

    #[test]
    fn regions_without_link_stylesheets_are_left_out_of_lc() {
        let root = lazy_fragment_id("app-shell", 0);
        let nested = lazy_fragment_id("app-shell", 1);
        let mut protocol = styled_fixture(CssStrategy::Link, &["theme-picker"]);
        emit(&mut protocol).unwrap();
        let owner = template_json(&protocol, "app-shell");
        assert!(
            owner.contains(r#""lp":"#) && !owner.contains(r#""lc""#),
            "{owner}"
        );
        let region = template_json(&protocol, &root);
        assert!(
            region.contains(&format!(r#""lc":{{"{nested}":["theme-picker.css"]}}"#)),
            "{region}"
        );

        let mut unstyled = fixture();
        emit(&mut unstyled).unwrap();
        for id in ["app-shell", root.as_str()] {
            let json = template_json(&unstyled, id);
            assert!(
                json.contains(r#""lp":"#) && !json.contains(r#""lc""#),
                "{json}"
            );
        }
    }

    #[test]
    fn style_and_module_packs_carry_css_text_and_list_no_stylesheets() {
        let styled = ["shared-badge", "settings-form", "theme-picker"];
        for strategy in [CssStrategy::Style, CssStrategy::Module] {
            let mut protocol = styled_fixture(strategy, &styled);
            let packs = emit(&mut protocol).unwrap();
            assert_eq!(packs.len(), 2);
            assert!(packs
                .iter()
                .any(|pack| pack.content.contains(".settings-form { color: red; }")));
            let templates = protocol
                .components
                .values()
                .map(|component| &component.template_json)
                .chain(
                    protocol
                        .client_fragments
                        .values()
                        .map(|fragment| &fragment.template_json),
                );
            for json in templates {
                assert!(!json.contains(r#""lc""#), "{json}");
            }
        }
    }

    #[test]
    fn regions_without_a_declaring_condition_are_invalid_metadata() {
        let mut protocol = fixture();
        protocol
            .components
            .get_mut("app-shell")
            .unwrap()
            .template_json = r#"{"h":"<p></p>","v":3}"#.to_owned();
        let error = emit(&mut protocol).unwrap_err();
        assert!(matches!(error, HandlerError::Invariant(_)), "{error}");
        assert!(error.to_string().contains("app-shell::lazy:@0"), "{error}");
    }

    #[test]
    fn file_names_are_content_addressed_and_filesystem_safe() {
        let first = file_name("app-shell", "{}");
        assert_eq!(first, file_name("app-shell", "{}"));
        assert_ne!(first, file_name("app-shell", "{ }"));
        assert!(first.starts_with("app-shell.") && first.ends_with(".json"));
        assert_eq!(first.len(), "app-shell.".len() + 16 + ".json".len());
        assert!(file_name("päge:x", "{}").starts_with("p_ge_x."));
    }

    #[test]
    fn pack_paths_are_relative_to_the_output_directory() {
        let name = file_name("app-shell", "{}");
        let path = pack_path(&name);
        assert_eq!(path, format!("_webui/packs/{name}"));
        assert!(!path.starts_with('/'));
    }
}
