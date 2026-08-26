# Plugins

WebUI provides a framework-agnostic plugin system that extends both the parser (build time) and the handler (render time). Plugins let framework authors customize WebUI's behavior - component discovery, attribute filtering, hydration marker injection - without modifying WebUI internals.

## How Plugins Work

The plugin system operates at three stages:

```
Discovery Plugin        Parser Plugin          Handler Plugin
┌──────────────────┐    ┌──────────────────┐    ┌──────────────────┐
│ Map package files│    │ Classify attrs   │    │ Inject hydration │
│ Pair HTML + CSS  │ -> │ Compile templates│ -> │ Process metadata │
│ Return components│    │ Emit metadata    │    │ Render bindings  │
└──────────────────┘    └──────────────────┘    └──────────────────┘
```

Discovery plugins map supported local or npm package layouts to components.
Parser plugins compile those components for their matching handler plugin.

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
| `fast` | Deprecated compatibility alias for `fast-v2` |
| `fast-v2` | FAST hydration plugin pinned to FAST major version 2 |
| `fast-v3` | FAST hydration plugin pinned to FAST major version 3 |
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

The Rust FAST integrations are `fast_v2` and `fast_v3`; their CLI names are
`fast-v2` and `fast-v3`. The deprecated `fast` identifier remains an alias for
`fast-v2`. Each versioned integration is pinned to the corresponding FAST major
version:

```bash
webui build ./src --out ./dist --plugin=fast-v3
webui serve ./src --state ./data/state.json --plugin=fast-v3 --watch
```

When a FAST plugin is selected, it recognizes an HTML file authored as one
wrapping `<f-template>`. With no plugin, or with the `webui` plugin,
`<f-template>` markup passes through like any other HTML:

```html
<!-- src/components/file-card.html -->
<f-template name="named-card" shadowrootmode="open">
  <template @click="{select($e)}">
    {{styles}}
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
of deriving `file-card` from the filename. If `name` is absent or contains only
whitespace, WebUI keeps the filename-derived tag after removing the generated
`.template.html` suffix. The wrapper must contain one direct inner `<template>`
as its only meaningful child: only whitespace and comments may surround it. A
meaningful sibling around the inner `<template>` and unsupported FAST syntax
fail the build rather than being silently dropped. Nested inert `<template>`
elements inside that direct child remain normal component content. Multiple
`<f-template>` elements are not currently supported and have a dedicated
authoring diagnostic.

Because the authored `<f-template name>` supplies the registry key, recursive
discovery also accepts a file whose stem is not itself a custom-element name.
FAST's generated files are named `<component>.template.html`; FAST discovery
removes that suffix before applying the filename fallback. An authored
`name="custom-*"` can still override it. Without a FAST plugin, a file is
considered only by the native hyphenated filename rule.

FAST npm packages use their Custom Elements Manifest as the component index.
For each `modules[].declarations[]` entry with a `tagName`, discovery maps the
module to a sibling `<component>.template.html` file. Generated packages may
use a virtual CEM module path; when that JavaScript path is absent, discovery
also checks component directories derived from the declaration class name,
including compact names such as `textarea` and the terminal class noun.
Optional styles may use `<component>.styles.css` or `<component>.css`. The
authored `<f-template name>` determines the final component name. Normal
component name, duplicate, CSS, and render-policy validation still applies.
FAST 2 and FAST 3 share this package layout.

The wrapper accepts only `name` and declarative-shadow-root options - attributes
beginning with `shadowroot` such as `shadowrootmode` and
`shadowrootdelegatesfocus`; any other wrapper attribute is a build error rather
than being silently dropped. Those options apply to the declarative Shadow root
and remain available to the FAST runtime. A leading `{{styles}}` marker
generated immediately after the inner `<template>` opening is reserved for
component CSS injection. A `{{styles}}` interpolation elsewhere remains a
normal binding.

Supported FAST authoring syntax includes:

- `<f-repeat value="{{item in items}}">` for repetition. It may include
  `positioning="true"` when the FAST client template uses positioning data.
- `<f-when value="{{condition}}">` for conditions, including quoted string
  literals.
- Text interpolation and boolean bindings.
- Client bindings `@event`, `:property`, `f-ref`, `f-slotted`, and
  `f-children`, including bindings on the root `<template>`.

`<f-when>` accepts only `value`; `<f-repeat>` accepts `value` and optional
`positioning`. Unsupported `f-*` constructs, other directive attributes, and
stray FAST closing tags fail the build. Markup-shaped text inside `<script>`
and `<style>` is treated as text rather than FAST syntax. Element and attribute names are
ASCII-case-insensitive, matching HTML semantics.

Multiple wrappers report `unsupported-multiple-f-templates`; malformed or
unsupported FAST declarative syntax reports `invalid-fast-template`, while
unclosed markup uses the shared `unclosed-html-tag` diagnostic.

Both plugins produce hydration output compatible with their pinned FAST major
version. WebUI-owned complex properties such as `:items="{{items}}"` populate
component scope during SSR but are not serialized as HTML attributes or copied
into FAST client state. FAST components must obtain their client state through
their own store, request, or other application mechanism and assign properties
used by the retained template before calling `super.connectedCallback()`.
Components with asynchronous state should delay that call until the state is
ready. Descendant property bindings then propagate those component-owned values
during hydration.

```ts
connectedCallback(): void {
  this.items = applicationStore.currentItems();
  super.connectedCallback();
}
```

For asynchronous sources, start loading from the component and call
`super.connectedCallback()` only after the result is assigned and the element
is still connected. Guard initialization so reconnection does not discard
interactive state.

## Writing Custom Plugins

To create a custom integration, implement the public discovery, parser, and/or
handler plugin traits.

### DiscoveryPlugin Trait

The `microsoft-webui-discovery` crate resolves source roots before passing them
to `DiscoveryPlugin`. Return `DiscoveredComponent` values to use WebUI's normal
component validation and compilation:

```rust
pub trait DiscoveryPlugin {
    fn cache_namespace(&self) -> &'static str;
    fn discover_local(&self, root: &Path) -> Result<Vec<DiscoveredComponent>>;
    fn package_cache_files(
        &self,
        package: PackageContext<'_>,
    ) -> Result<Vec<PathBuf>>;
    fn discover_package(
        &self,
        package: PackageContext<'_>,
    ) -> Result<Vec<DiscoveredComponent>>;
}
```

Call `discover_source_with_plugin(source, search_dir, plugin)` to resolve a
source with a custom layout. `package_cache_files` must return deterministic
paths for every package file that can affect discovery, including optional
files that do not yet exist.

### ParserPlugin Trait

```rust
pub type ComponentSourceTransform =
    for<'a> fn(ComponentSource<'a>) -> Result<Option<TransformedComponentSource>>;

pub struct ComponentProcessing {
    pub source_transform: Option<ComponentSourceTransform>,
    pub process_root_template_attributes: bool,
    pub inline_styles_after_content: bool,
}

pub struct ComponentBuildContext<'a> {
    pub component: &'a Component,
    pub template: &'a str,
    pub uses_shadow_dom: bool,
    pub style: Option<ComponentStyleDelivery<'a>>,
}

pub trait ParserPlugin {
    fn configure_parser(&mut self, options: &ParserOptions) {}
    fn component_processing(&self) -> ComponentProcessing { ComponentProcessing::default() }
    fn begin_fragment(&mut self, context: FragmentContext<'_>) {}
    fn component_built(&mut self, context: ComponentBuildContext<'_>) -> Result<()> { Ok(()) }
    fn process_attribute(&mut self, context: AttributeContext<'_>) -> AttributeAction { AttributeAction::Keep }
    fn finish_opening_tag(&mut self, context: ElementStartContext<'_>) -> Option<Vec<u8>> { None }
    fn finish(self: Box<Self>) -> Result<ParserPluginArtifacts> { Ok(ParserPluginArtifacts::None) }
}
```

`component_processing` is read once, so static component behavior does not add a
virtual call per component. A missing source transform avoids the indirect call
entirely, and a transform returns `Ok(None)` to preserve source without
allocation. The remaining callbacks follow parser lifecycle order and have
no-op defaults, so a plugin implements only the phases it owns.

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
