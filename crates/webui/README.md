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
// result.stats — build timing and fragment counts
```

## API

### Build

| Function | Description |
|----------|-------------|
| `build(options)` | Parse templates, discover components, compile protocol |
| `build_to_disk(options, out_dir)` | Build and write `protocol.bin` + CSS to disk |

```rust
use webui::{build_to_disk, BuildOptions, CssStrategy, DomStrategy};

build_to_disk(
    BuildOptions {
        app_dir: "src".into(),
        entry: "index.html".into(),
        css: CssStrategy::Link,        // or CssStrategy::Style for inline
        dom: DomStrategy::Shadow,      // or DomStrategy::Light for light DOM
        plugin: Some("fast".into()),    // FAST hydration plugin
        components: vec![],             // additional component sources
    },
    Path::new("dist"),
)?;
```

### Render

```rust
use webui::{WebUIHandler, WebUIProtocol, ResponseWriter, RenderOptions};

let protocol = WebUIProtocol::from_protobuf(&protocol_bytes)?;
let state: serde_json::Value = serde_json::json!({"name": "WebUI"});

let handler = WebUIHandler::new();
handler.handle(&protocol, &state, &RenderOptions::new("index.html", "/"), &mut writer)?;
```

With the FAST hydration plugin:

```rust
use webui::{WebUIHandler, HandlerPlugin};
use webui_handler::plugin::fast::FastHydrationPlugin;

let handler = WebUIHandler::with_plugin(|| Box::new(FastHydrationPlugin::new()));
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
use webui_handler::route_handler;

let partial = route_handler::render_partial(
    &protocol, state, "index.html", "/users/42", inventory_hex,
);
// Returns: { state, templates, inventory, path, chain }
```

## Types

| Type | Description |
|------|-------------|
| `BuildOptions` | Build configuration (app_dir, entry, css, plugin, components) |
| `BuildResult` | Build output (protocol, css_files, component_templates, stats) |
| `BuildStats` | Build metrics (duration, fragment_count, protocol_size_bytes) |
| `WebUIProtocol` | Compiled protocol (from protobuf binary) |
| `WebUIHandler` | Rendering engine (stateless, thread-safe) |
| `RenderOptions` | Render configuration (entry_id, request_path) |
| `ResponseWriter` | Trait for streaming rendered output |
| `CssStrategy` | CSS delivery mode (Link or Style) |
| `WebUIError` | Error type for build/inspect operations |

## License

MIT
