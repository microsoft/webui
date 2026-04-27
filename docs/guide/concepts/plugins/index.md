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

Parser plugins emit opaque binary data into `Plugin` protocol fragments. Handler plugins receive that data at render time via `on_plugin_data`. WebUI never interprets this data - each plugin pair defines its own contract.

## Using Plugins via the CLI

Pass `--plugin <NAME>` to `webui build` or `webui serve`:

```bash
# Build with the FAST plugin
webui build ./my-app --out ./dist --plugin=fast

# Dev server with the FAST plugin
webui serve ./my-app --state ./data/state.json --plugin=fast
```

When `--plugin=fast` is specified:
- **Build**: The `FastParserPlugin` is loaded during parsing
- **Start**: Both `FastParserPlugin` and `FastHydrationPlugin` are loaded

## Using Plugins with Handlers

<webui-tabs>
<webui-tab slot="tab" active>Rust</webui-tab>
<webui-tab slot="tab">Node.js</webui-tab>
<webui-tab slot="tab">FFI (C API)</webui-tab>
<webui-tab-panel active>

```rust
use webui_handler::plugin::fast::FastHydrationPlugin;
use webui::WebUIHandler;

let handler = WebUIHandler::with_plugin(|| Box::new(FastHydrationPlugin::new()));
handler.handle(&protocol, &state, &options, &mut writer)?;
```

</webui-tab-panel>
<webui-tab-panel>

```js
import { renderStream } from '@microsoft/webui';

renderStream(protocolData, state, (chunk) => res.write(chunk), 'fast');
```

</webui-tab-panel>
<webui-tab-panel>

```c
void *handler = webui_handler_create_with_plugin("fast");
char *html = webui_handler_render(handler, protocol_data, protocol_len, state_json);
```

</webui-tab-panel>
</webui-tabs>

## Built-in Plugin: FAST-HTML

The `fast` plugin provides server-side rendering support for [FAST-HTML](https://github.com/nicholasgasior/fast-html) components with client-side hydration.

### Parser Side (`FastParserPlugin`)

During `webui build --plugin=fast`, the parser plugin:

- **Skips framework attributes**: `@click`, `f-ref`, `f-slotted`, `f-children` are removed from the protocol (they're handled client-side)
- **Counts dynamic bindings**: Emits binding counts per element as `Plugin` fragments for the handler
- **Tracks components**: Records all custom elements discovered during parsing
- **Injects `<f-template>` wrappers**: At `</body>`, injects template wrappers for each component with FAST syntax conversion

#### Syntax Conversion

The plugin converts WebUI template syntax to FAST equivalents inside `<f-template>` blocks:

| WebUI Syntax | FAST Syntax |
|-------------|-------------|
| `<if condition="EXPR">` | `<f-when value="{EXPR}">` |
| `<for each="item in items">` | `<f-repeat value="{items}">` |
| `{{expr}}` in `:attr` values | `{expr}` |

### Handler Side (`FastHydrationPlugin`)

During rendering with `--plugin=fast`, the handler plugin injects HTML comment markers that FAST-HTML's client-side runtime uses to locate and re-hydrate dynamic content:

| Context | Start Marker | End Marker |
|---------|-------------|------------|
| Signal / If / For | `<!--fe-b$$start$$INDEX$$NAME$$fe-b-->` | `<!--fe-b$$end$$INDEX$$NAME$$fe-b-->` |
| For-loop item | `<!--fe-repeat$$start$$INDEX$$fe-repeat-->` | `<!--fe-repeat$$end$$INDEX$$fe-repeat-->` |

For attribute bindings, data attributes are emitted instead:

| Type | Attribute |
|------|-----------|
| Single binding | `data-fe-b-INDEX` |
| Multiple bindings | `data-fe-c-INDEX-COUNT` |

The plugin maintains per-scope binding counters that reset when entering components or loop items.

## Built-in Plugin: WebUI Framework

The `webui` plugin provides server-side rendering support for [WebUI Framework](https://github.com/microsoft/webui) components with automatic hydration.

### Parser Side (`WebUIParserPlugin`)

During `webui build --plugin=webui`, the parser plugin:

- **Skips framework attributes**: `@click`, `@keydown`, `w-ref`, and other event/ref bindings are removed from the protocol (handled client-side)
- **Emits binding metadata**: 12-byte `Plugin` fragments encoding `[binding_count, event_start, event_count]` per element
- **Tracks components**: Records custom elements for template metadata generation
- **Compiles templates**: Generates optimized metadata as raw JS IIFE strings registered in `window.__webui.templates` (wrapped in `<script>` for SSR, evaluated directly for SPA navigation)

### Handler Side (`WebUIHydrationPlugin`)

During rendering with `--plugin=webui`, the handler injects lightweight comment markers for structural boundaries:

| Context | Marker | Example |
|---------|--------|---------|
| Repeat block | `<!--wr-->` / `<!--/wr-->` | Wraps the entire `<for>` loop |
| Repeat item | `<!--wi-->` | Before each loop iteration |
| Conditional block | `<!--wc-->` / `<!--/wc-->` | Wraps the `<if>` block content |

Text bindings, attribute bindings, and event handlers need no SSR markers - the client resolves them from compiled metadata path indices.

### Using the WebUI Plugin

```bash
# Build with WebUI Framework hydration
webui build ./src --out ./dist --plugin=webui

# Dev server with WebUI Framework
webui serve ./src --state ./data/state.json --plugin=webui --watch
```

```rust
// Rust handler
use webui_handler::plugin::webui::WebUIHydrationPlugin;
let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
```

## Writing Custom Plugins

To create a custom plugin, implement the `ParserPlugin` and/or `HandlerPlugin` traits:

### ParserPlugin Trait

```rust
pub trait ParserPlugin {
    /// Called when a custom element is encountered.
    fn on_parse_component(&mut self, tag_name: &str, component: &Component) -> Result<()>;

    /// Return `true` to skip an attribute (it won't appear in the protocol).
    fn should_skip_attribute(&self, attr_name: &str) -> bool;

    /// Called before the body_end signal. Return HTML to inject as a raw fragment.
    fn on_body_end(&mut self) -> Option<String>;

    /// Called after all attributes on an element are processed.
    /// Return opaque bytes to emit as a Plugin protocol fragment.
    fn on_element_parsed(&mut self, binding_attribute_count: u32) -> Option<Vec<u8>>;
}
```

### HandlerPlugin Trait

```rust
pub trait HandlerPlugin {
    /// Enter a new scope (component or loop item).
    fn push_scope(&mut self);
    /// Leave the current scope.
    fn pop_scope(&mut self);

    /// Called before/after a dynamic binding (signal, for, if).
    fn on_binding_start(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()>;
    fn on_binding_end(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()>;

    /// Called before/after each item in a for-loop.
    fn on_repeat_item_start(&mut self, index: usize, writer: &mut dyn ResponseWriter) -> Result<()>;
    fn on_repeat_item_end(&mut self, index: usize, writer: &mut dyn ResponseWriter) -> Result<()>;

    /// Process opaque data from a Plugin protocol fragment.
    fn on_plugin_data(&mut self, data: &[u8], writer: &mut dyn ResponseWriter) -> Result<()>;
}
```

## Next Steps

- [CLI Reference](/guide/cli/) - `--plugin` flag details
- [Rust Handler](/guide/integrations/rust) - Using plugins with the Rust handler
- [Hello World Tutorial](/tutorials/hello-world) - Basic WebUI app
