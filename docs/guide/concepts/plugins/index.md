# Plugins

WebUI provides a framework-agnostic plugin system that extends both the parser (build time) and the handler (render time). Plugins let framework authors customize WebUI's behavior — component discovery, attribute filtering, hydration marker injection — without modifying WebUI internals.

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

Parser plugins emit opaque binary data into `Plugin` protocol fragments. Handler plugins receive that data at render time via `on_plugin_data`. WebUI never interprets this data — each plugin pair defines its own contract.

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

::: code-group
```rust [Rust]
use webui_handler::plugin::FastHydrationPlugin;
use webui::WebUIHandler;

let mut handler = WebUIHandler::with_plugin(Box::new(FastHydrationPlugin::new()));
handler.handle(&protocol, &state, &mut writer)?;
```

```js [Node.js]
import { renderStream } from '@microsoft/webui';

renderStream(protocolData, state, (chunk) => res.write(chunk), 'fast');
```

```c [FFI (C API)]
void *handler = webui_handler_create_with_plugin("fast");
char *html = webui_handler_render(handler, protocol_data, protocol_len, state_json);
```
:::

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

- [CLI Reference](/guide/cli/) — `--plugin` flag details
- [Rust Handler](/guide/concepts/handlers/rust) — Using plugins with the Rust handler
- [Hello World Tutorial](/tutorials/hello-world) — Basic WebUI app
