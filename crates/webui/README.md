# webui

Programmatic Rust API for the [WebUI](https://github.com/microsoft/webui) build-time rendering framework. Parse templates, compile protocols, and render HTML — no CLI required.

## Install

```bash
cargo add webui
```

## Quick Start

```rust
use webui::{build, BuildOptions, DomStrategy};

// Build a WebUI application from an app directory
let result = build(BuildOptions {
    app_dir: "my-app/src".into(),
    entry: "index.html".into(),
    dom: DomStrategy::Shadow,
    ..Default::default()
})?;

// result.protocol_bytes — serialized protocol (protobuf binary)
// result.css_files — extracted component CSS files
// result.component_asset_files — static `.webui.js` ESM component assets
// result.stats — build timing and fragment counts
```

## API

### Build

| Function | Description |
|----------|-------------|
| `build(options)` | Parse templates, discover components, compile protocol |
| `build_to_disk(options, out_dir)` | Build and write `protocol.bin`, CSS, and component assets to disk |

```rust
use webui::{build_to_disk, BuildOptions, CssStrategy, DomStrategy, LegalComments, Plugin};

build_to_disk(
    BuildOptions {
        app_dir: "src".into(),
        entry: "index.html".into(),
        css: CssStrategy::Link,        // or CssStrategy::Style for inline
        dom: DomStrategy::Shadow,      // or DomStrategy::Light for light DOM
        plugin: Some(Plugin::FastV3),    // @microsoft/fast-element 3.x hydration plugin
        legal_comments: LegalComments::Inline, // preserve legal CSS comments
        components: vec![],             // additional component sources
        ..BuildOptions::default()
    },
    Path::new("dist"),
)?;
```

For CDN/cache-friendly Link-mode CSS and static component assets, override the
asset output fields:

```rust
BuildOptions {
    app_dir: "src".into(),
    css_file_name_template: "[name]-[hash].[ext]".into(),
    css_public_base: Some("https://cdn.example.com/assets".into()),
    ..BuildOptions::default()
}
```

To emit lazy static component assets from Rust, set `component_asset_roots` and
use the WebUI plugin:

```rust
BuildOptions {
    app_dir: "src".into(),
    plugin: Some(Plugin::WebUI),
    component_asset_roots: vec!["settings-dialog".into()],
    ..BuildOptions::default()
}
```

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
    state_json, "index.html", "/users/42", inventory_hex,
)?;
// Returns: { state, templates, inventory, path, chain }
```

## Types

| Type | Description |
|------|-------------|
| `BuildOptions` | Build configuration (app_dir, entry, css, plugin, components, css_file_name_template, css_public_base) |
| `BuildResult` | Build output (protocol, css_files, component_templates with metadata/closures, stats) |
| `BuildStats` | Build metrics (duration, fragment_count, protocol_size_bytes) |
| `Protocol` | Loaded immutable runtime protocol with reusable indices |
| `WebUIHandler` | Rendering engine (stateless, thread-safe) |
| `RenderOptions` | Render configuration (entry_id, request_path) |
| `ResponseWriter` | Trait for streaming rendered output |
| `CssStrategy` | CSS delivery mode (Link or Style) |
| `WebUIError` | Error type for build/inspect operations |

## License

MIT
