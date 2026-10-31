# webui

Programmatic Rust API for the [WebUI](https://github.com/microsoft/webui) build-time rendering framework. Parse templates, compile protocols, and render HTML — no CLI required.

## Install

```bash
cargo add webui
```

## Quick Start

```rust
use webui::{build, BuildOptions};

// Build a WebUI application from an app directory
let result = build(BuildOptions {
    app_dir: "my-app/src".into(),
    entry: "index.html".into(),
    ..Default::default()
})?;

// result.protocol_bytes — serialized protocol (protobuf binary)
// result.css_files — extracted component CSS files
// result.component_asset_files — root/chunk `.webui.js` assets
// result.lazy_pack_files — lazy template packs with their region cost data
// result.lazy_scripts_module — generated `lazy.webui.js` and its `lazy.webui.d.ts` when lazy regions exist
// result.metafile — optional metafile JSON (esbuild-compatible graph plus `lazyPacks`)
// result.stats — build timing and fragment counts
```

## API

### Build

| Function | Description |
|----------|-------------|
| `build(options)` | Parse templates, discover components, compile protocol |
| `build_to_disk(options, out_dir)` | Build and write `protocol.bin`, CSS, and lazy packs to `out_dir`, and component ESM inputs, `lazy.webui.js`, and `lazy.webui.d.ts` to `modules_out` (default `out_dir`) |

```rust
use webui::{build_to_disk, BuildOptions, CssStrategy, DomStrategy, LegalComments, Plugin};

build_to_disk(
    BuildOptions {
        app_dir: "src".into(),
        entry: "index.html".into(),
        css: CssStrategy::Link,        // or CssStrategy::Style for inline
        dom: DomStrategy::Light,       // opt into global Light + authored Shadow islands
        plugin: Some(Plugin::FastV3),    // @microsoft/fast-element 3.x hydration plugin
        legal_comments: LegalComments::Inline, // preserve legal CSS comments
        components: vec![],             // additional component sources
        ..BuildOptions::default()
    },
    Path::new("dist"),
)?;
```

Unwrapped components default to a generated open Shadow root. Set
`dom: DomStrategy::Light` to render unwrapped components as authored/global
Light DOM. A sole bare `<template>` is also an explicit Light wrapper and is
unwrapped; a sole authored `<template shadowrootmode="open">` remains Shadow.
Light CSS must use ordinary selectors: `:host`, `:host-context`, and `::slotted`
are rejected with `unsupported-light-css`.

For CDN/cache-friendly Link-mode CSS, override the asset output fields:

```rust
BuildOptions {
    app_dir: "src".into(),
    css_file_name_template: "[name]-[hash].[ext]".into(),
    asset_base: Some("https://cdn.example.com/assets/".into()),
    ..BuildOptions::default()
}
```

`asset_base` is a URL or path ending in `/` that maps to the output directory.
Build output keeps base-relative paths; the handler prefixes stylesheet links
with the base when it renders and passes it to the browser. Module preload
hints keep their paths, because they name your own scripts. Override the base
per request with the reserved `{"$webui": {"assetBase": "..."}}` state member.

To emit lazy static component assets from Rust, set `component_asset_roots` and
use the WebUI plugin:

```rust
BuildOptions {
    app_dir: "src".into(),
    plugin: Some(Plugin::WebUI),
    component_asset_roots: vec![
        "settings-dialog".into(),
        "mail-thread".into(),
    ],
    metafile: true,
    ..BuildOptions::default()
}
```

The result contains one stable module per asset-owned component plus a thin
root entry with static imports for its closure. These are application-bundler
inputs: the bundler owns final chunks, hashes, public paths, and caching.
Entry-reachable dependencies remain external prerequisites. Asset-only
component records are removed from `protocol.bin`. Component assets cannot be
combined with `<route>`.

`<if lazy>` regions compile to template packs in `BuildResult::lazy_pack_files`
and to `BuildResult::lazy_scripts_module`, the generated `lazy.webui.js` that
maps each region to dynamic imports of the scripted components it introduces
and the page does not already load at startup. Import it from the application
entry so the bundler loads that code. Its `declaration` is `lazy.webui.d.ts`,
written next to it so TypeScript entries get the lazy runtime's types without
`allowJs`. Set `modules_out` to the directory they will be written to (relative
specifiers start there; `build_to_disk` defaults it to `out_dir`), and
`component_modules` for components without a `.ts` or `.js` sibling module.

`LegalComments::Inline` is the default and preserves legal CSS comments
containing `@license` or `@preserve`, or starting with `/*!` or `//!`. Use
`LegalComments::None` to strip all HTML and CSS comments from build output.

### Render

```rust
use webui::{Protocol, RenderOptions, ResponseWriter, WebUIHandler};

let protocol = Protocol::from_protobuf(&protocol_bytes)?;
let state: serde_json::Value = serde_json::json!({"name": "WebUI"});

let handler = WebUIHandler::new();
handler.render(&protocol, &state, &RenderOptions::new("index.html", "/"), &mut writer)?;
```

With a hydration plugin enabled (the `webui` plugin shown here; see the WebUI documentation for the available plugin identifiers):

```rust
use webui::{WebUIHandler, HandlerPlugin};
use webui_handler::plugin::webui::WebUIHydrationPlugin;

let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
```

### Progressive Responses

For host-paced `<boundary>` rendering, drive the four-state session directly:

```rust
use webui::{BoundaryMode, RenderOptions};

let options = RenderOptions::new("index.html", "/");
let mut response =
    handler.stream_response(&protocol, &options, &mut writer)?;
let mut step = response.start(&initial_state)?;

while !step.done {
    step = match step.boundary.as_ref() {
        Some(boundary) => {
            let state =
                load_state(&boundary.owner, &boundary.name, boundary.key.as_ref())?;
            response.resume(boundary.instance_id, &state, BoundaryMode::Final)?
        }
        None => response.advance()?,
    };
}
```

| Method | Result |
|---|---|
| `start(state)` | Shell bytes through the first descriptor or terminal |
| `resume(instance_id, state, mode)` | Only the pending occurrence through its checkpoint |
| `advance()` | Following parent bytes through the next descriptor or terminal |
| `update(instance_id, patch)` | Projected state for an updatable occurrence |

A descriptor means call `resume`; no descriptor with `done == false` means call
`advance`; `done == true` means complete. Boundary-only `resume` makes a
checkpoint independently flushable. `advance` writes following parent or tail
bytes, so no sibling boundary is needed. An update is valid between `resume` and
`advance`.

### Inspect

```rust
use webui::{inspect, inspect_bytes};

// From a file
let json = inspect(Path::new("dist/protocol.bin"))?;

// From bytes
let json = inspect_bytes(&protocol_bytes)?;
```

### Partial Responses (Client Navigation)

For servers handling client-side navigation, produce a complete JSON partial:

```rust
let partial = protocol.render_partial(
    state, "index.html", "/users/42", inventory_hex,
)?;
// Returns: { state, templates, inventory, path, chain }
```

Serialized host boundaries can call `render_partial_json()` to validate and
project raw JSON without materializing a duplicate state tree:

```rust
let partial = protocol.render_partial_json(
    state_json, "index.html", "/users/42", inventory_hex,
)?;
```

## Types

| Type | Description |
|------|-------------|
| `BuildOptions` | Build configuration (app_dir, entry, css, plugin, components, css_file_name_template, asset_base, component_modules, modules_out) |
| `BuildResult` | Build output (protocol, css_files, component_templates with metadata/closures, lazy_pack_files, lazy_scripts_module, stats) |
| `BuildStats` | Build metrics (duration, fragment_count, protocol_size_bytes) |
| `Protocol` | Loaded immutable runtime protocol with reusable indices |
| `WebUIHandler` | Rendering engine (stateless, thread-safe) |
| `RenderOptions` | Render configuration (entry_id, request_path) |
| `ResponseWriter` | Trait for streaming rendered output |
| `CssStrategy` | CSS delivery mode (Link, Style, or Module) |
| `WebUIError` | Error type for build/inspect operations |

## License

MIT
