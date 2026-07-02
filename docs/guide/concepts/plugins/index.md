# Plugins

WebUI provides a framework-agnostic plugin system that extends both the parser (build time) and the handler (render time). Plugins let framework authors customize WebUI's behavior - component discovery, attribute filtering, hydration marker injection - without modifying WebUI internals.

## How Plugins Work

The plugin system operates at two stages:

```
Build time (Parser Plugin)         Runtime (Handler Plugin)
┌──────────────────────────┐       ┌──────────────────────────┐
│ Skip framework attrs     │       │ Inject hydration markers │
│ Track components         │  ───► │ Manage scope counters    │
│ Emit opaque Plugin data  │       │ Process Plugin data      │
│ Inject content at </body>│       │ Wrap bindings/repeats    │
└──────────────────────────┘       └──────────────────────────┘
```

Parser plugins emit opaque binary data into `Plugin` protocol fragments. Handler plugins receive that data at render time via `on_element_data`. WebUI never interprets this data - each plugin pair defines its own contract.

## Using Plugins via the CLI

Pass `--plugin <NAME>` to `webui build` or `webui serve`:

```bash
# Build with a named plugin
webui build ./my-app --out ./dist --plugin=<name>

# Dev server with a named plugin
webui serve ./my-app --state ./data/state.json --plugin=<name>
```

When a plugin is selected, both its parser-side and (for `serve`) handler-side
implementations are loaded.

### Built-in plugin identifiers

| Name | Behavior |
|------|----------|
| `fast` | Deprecated alias for `fast-v2` |
| `fast-v2` | Deprecated FAST 2 compatibility name |
| `fast-v3` | FAST 3 hydration plugin |
| `webui` | WebUI framework hydration plugin |

## Using Plugins with Handlers

<webui-press-tabs>
<webui-press-tab slot="tab" active>Rust</webui-press-tab>
<webui-press-tab slot="tab">Node.js</webui-press-tab>
<webui-press-tab slot="tab">FFI (C API)</webui-press-tab>
<webui-press-tab-panel active>

```rust
use webui::WebUIHandler;

let handler = WebUIHandler::with_plugin(|| Box::new(MyHydrationPlugin::new()));
handler.render(&protocol, &state, &options, &mut writer)?;
```

</webui-press-tab-panel>
<webui-press-tab-panel>

```js
import { Protocol } from '@microsoft/webui';

const protocol = new Protocol(protocolData, { plugin: '<name>' });
protocol.renderStream(state, (chunk) => res.write(chunk));
```

</webui-press-tab-panel>
<webui-press-tab-panel>

```c
void *handler = webui_handler_create_with_plugin("<name>");
webui_protocol_t *protocol = webui_protocol_create(protocol_data, protocol_len);
char *html = webui_handler_render(handler, protocol, state_json, "index.html", "/");
```

</webui-press-tab-panel>
</webui-press-tabs>

### Using the WebUI Plugin

```bash
# Build with WebUI Framework hydration
webui build ./src --out ./dist --plugin=webui \
  --projection-manifest ./dist/webui-projection.json

# Dev server with WebUI Framework
webui serve ./src --state ./data/state.json --plugin=webui \
  --projection-manifest ./dist/webui-projection.json --watch
```

Projection is optional. Without a manifest, the WebUI plugin preserves full
state. When a manifest is supplied, coverage is strict and the manifest must be
produced by the completed browser bundle. See
[Build-Time State Projection](/guide/concepts/hydration#build-time-state-projection).

```rust
// Rust handler
use webui_handler::plugin::webui::WebUIHydrationPlugin;
let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
```

### Using FAST Plugins

The built-in FAST plugins are `fast`, `fast-v2`, and `fast-v3`:

```bash
webui build ./src --out ./dist --plugin=fast-v3
webui serve ./src --state ./data/state.json --plugin=fast-v3 --watch
```

For FAST builds, a component HTML file may be authored as one wrapping
`<f-template>`:

```html
<!-- src/components/file-card.html -->
<f-template name="named-card">
  <template>
    <f-when value="{{visible}}">
      <f-repeat value="{{item in items}}">
        <button @click="{select(item)}" :config="{config}">
          {{item.label}}
        </button>
      </f-repeat>
    </f-when>
  </template>
</f-template>
```

`<f-template name="named-card">` registers the component as `named-card` instead
of deriving `file-card` from the filename. If the `name` attribute is omitted,
the filename-derived tag is used. Multiple `<f-template>` elements in one
component source are unsupported and fail the build with an authoring error.

WebUI uses two views of this source:

- **SSR parse view:** the inner FAST declarative template is converted to WebUI
  syntax for build-time parsing. `<f-repeat value="{{item in items}}">` becomes
  `<for each="item in items">`; `<f-when value="{{condition}}">` becomes
  `<if condition="condition">`; directive values are unwrapped; client-only FAST
  syntax that does not affect server-rendered output is removed, including
  `@event`, `:prop`, `f-ref`, `f-slotted`, `f-children`, and similar
  client-only directives. FAST plugins receive parser-only binding markers for
  those stripped directives so hydration binding indexes remain aligned.
- **Client artifact view:** the authored FAST template is preserved and emitted
  as the client `<f-template>` payload. It still goes through normal component
  processing and normalization, including wrapper handling and CSS injection for
  the selected CSS strategy. Non-FAST plugins receive the normal WebUI parse
  view instead of FAST artifacts.

## Writing Custom Plugins

To create a custom plugin, implement the `ParserPlugin` and/or `HandlerPlugin` traits:

### ParserPlugin Trait

```rust
pub struct ComponentTemplateContext {
    pub uses_shadow_dom: bool,
}

pub trait ParserPlugin {
    /// Called before parsing begins for a fragment.
    fn start_fragment(&mut self, fragment_id: &str) {}

    /// Called with the plugin-facing template and Shadow ownership metadata.
    /// Authored root `<template>` attributes are preserved for plugins.
    fn register_component_template(
        &mut self,
        tag_name: &str,
        component: &Component,
        processed_template: &str,
        context: ComponentTemplateContext,
    ) -> Result<()>;

    /// Return true only for plugins that consume FAST `<f-template>` artifacts.
    fn uses_fast_template_artifacts(&self) -> bool {
        false
    }

    /// Decide how a framework-owned attribute should be handled.
    fn classify_attribute(&mut self, attr_name: &str) -> AttributeAction;

    /// Called after all attributes on an element are processed.
    /// Return opaque bytes to emit as a Plugin protocol fragment.
    fn finish_element(&mut self, binding_attribute_count: u32) -> Option<Vec<u8>>;

    /// Consume the plugin and return captured build artifacts.
    ///
    /// Returns an error if the plugin captured an invalid template construct
    /// (e.g. a malformed `@event` handler) while producing its artifacts.
    fn into_artifacts(self: Box<Self>) -> Result<ParserPluginArtifacts> {
        Ok(ParserPluginArtifacts::None)
    }
}
```

When producing a `ComponentTemplateArtifact`, pass
`context.uses_shadow_dom` to its constructor. This parser-derived boolean is
required metadata and is not inferred after the plugin returns its artifacts.

### HandlerPlugin Trait

```rust
pub trait HandlerPlugin: Send {
    /// Enter a new scope (component or loop item).
    fn push_scope(&mut self);
    /// Leave the current scope.
    fn pop_scope(&mut self);

    /// Called before/after a signal binding. `raw` is true only when the
    /// binding owns a replaceable HTML sibling range.
    fn on_binding_start(
        &mut self,
        name: &str,
        raw: bool,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()>;
    fn on_binding_end(
        &mut self,
        name: &str,
        raw: bool,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()>;

    /// Called before/after for-loop and if-condition blocks.
    fn on_for_start(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()>;
    fn on_for_end(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()>;
    fn on_if_start(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()>;
    fn on_if_end(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()>;

    /// Called before/after each item in a for-loop.
    fn on_repeat_item_start(&mut self, index: usize, writer: &mut dyn ResponseWriter) -> Result<()>;
    fn on_repeat_item_end(&mut self, index: usize, writer: &mut dyn ResponseWriter) -> Result<()>;

    /// Process opaque data from a Plugin protocol fragment.
    fn on_element_data(&mut self, data: &[u8], writer: &mut dyn ResponseWriter) -> Result<()>;

    /// Write framework-specific route component state attributes.
    fn write_route_component_state(
        &self,
        state: &serde_json::Value,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()>;
}
```

### Threading Contract

`HandlerPlugin` requires `Send`, but not `Sync`. WebUI creates a fresh plugin
instance for every render and never calls one instance concurrently, so
single-owner interior mutability such as `Cell` or `RefCell` is supported.

The `Send` bound also applies when your application only uses buffered
rendering. The same handler factory can open an owned `StreamingSession`, which
parks the live plugin between calls and may move between host threads. Rust
cannot safely recover `Send` after a factory result has been erased to a trait
object, so the guarantee must be part of the plugin implementation. Do not keep
thread-affine values such as `Rc` in a handler plugin; use owned values, `Arc`,
or another sendable handle.

## Next Steps

- [CLI Reference](/guide/cli/) - `--plugin` flag details
- [Rust Handler](/guide/integrations/rust) - Using plugins with the Rust handler
- [Hello World Tutorial](/tutorials/hello-world) - Basic WebUI app
