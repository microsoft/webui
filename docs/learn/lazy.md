---
title: "Lazy template bodies"
description: "Keep dialogs, popovers, and other conditional UI out of the first page, then load their compiled templates when they are needed."
---

# Lazy template bodies

Try the [popover and dialog example](https://github.com/microsoft/webui/tree/main/examples/app/lazy-ui):
`cargo xtask dev lazy-ui`. Its `lazy-popover` and `lazy-dialog` components own
native overlays, with all interior content in an inline lazy body.

Add `lazy` to an ordinary `<if>` when its UI should stay compiled but not be
delivered until it is needed. The body can contain native HTML, multiple roots,
conditions, repeats, refs, events, and component calls. It does not become a
component: state, handlers, refs, and loop variables keep their existing owner.

```html
<!-- app-shell.html -->
<button @click="{toggleSettings()}">Settings</button>

<if condition="settingsOpen" lazy>
  <section>
    <h2>Settings</h2>
    <p>{{displayName}}</p>
  </section>
</if>
```

```typescript
toggleSettings(): void {
  this.settingsOpen = !this.settingsOpen;
}
```

That is the whole API. The build compiles each lazy body into a static,
content-addressed **template pack**, and the page only references it. When the
condition becomes true, the framework fetches the pack and the scripts of the
components the body introduces, if they are not loaded yet, registers its
templates, and renders the body. You write no server endpoint, region id, or
loader code.

This is different from `w-hydrate="lazy"` and `w-render="lazy"`, which postpone
work on already server-rendered components. `<if lazy>` omits an inactive body's
markup and templates from the initial document.

## Choose when the template loads

The optional value of `lazy` sets when the pack is fetched ahead of use.
Prefetching only fetches and registers templates and loads the region's
component scripts; it never renders or requests application state.

| Declaration | Fetches the pack | Use for |
| --- | --- | --- |
| `lazy` or `lazy="intent"` | When a pointer moves over, focus enters, or a touch starts inside the owning component | Most dialogs, popovers, and panels behind a control |
| `lazy="idle"` | When the browser is idle after the page loads | UI that is likely to be used soon |
| `lazy="eager"` | Right after startup hydration | UI that is needed almost immediately |
| `lazy="demand"` | Only when the condition becomes true, or when code calls `$prefetch()` | Rare UI, UI prefetched from a specific control, and regions nested in other lazy regions |

Prefetching limits competition with the page's startup work:

- Eager and idle prefetches use low fetch priority. Intent and `$prefetch()` use
  low priority until the window `load` event and high priority after it. Opening
  a region always uses high priority.
- Component scripts load through your bundler's `import()`, which takes no
  priority hint, and browsers raise stylesheet preloads to high priority. So a
  prefetch before the window `load` event fetches only the pack and starts the
  region's stylesheets and scripts once the page has loaded. Opening a region
  loads everything immediately.
- While the browser prerenders the page (`document.prerendering`), nothing is
  prefetched; prefetching starts when the page is shown. Opening a region never
  waits.
- Idle and eager prefetch are skipped when the user has enabled data saving.

Focus that is already inside the owner when the page starts, such as an
`autofocus` search box whose owner holds its suggestions, also prefetches the
owner's regions, but only once the page has loaded and is idle. Initial focus
is not treated as an immediate gesture. A pointer, focus, or touch event inside
the owner before then prefetches at once, and opening a region bypasses these
speculative delays.

Prefer `intent` for UI that opens from a control in the owner. With `demand`, the
body waits for its pack when the condition becomes true unless the pack is
already available. A cold fetch adds a network round trip; loading from the
HTTP cache can still add asynchronous work. Prefetch ahead of use when that
first-open delay is unacceptable.

## Prefetch from code

`$prefetch()` starts loading a component's lazy regions before they open. It
returns immediately and never renders anything. Without an argument it covers
every lazy region in the component's template, including `demand` regions. With
a state path it covers only the regions whose condition reads that path or a
path below it, so `$prefetch('settings')` also covers `settings.open`.

Intent covers the whole owning component. When one large owner, such as a grid
of tiles, holds several regions, hovering anywhere in it would fetch all of them.
Scope the prefetch to the control that opens each region instead:

```html
<button @click="{openSettings()}"
  @pointerenter="{$prefetch('settingsOpen')}"
  @focus="{$prefetch('settingsOpen')}">Settings</button>

<if condition="settingsOpen" lazy="demand">
  <settings-panel></settings-panel>
</if>
```

Call it from code to load UI after your own milestone, such as when the page's
primary content is ready:

```typescript
onPrimaryContentReady(): void {
  this.$prefetch('settingsOpen');
}
```

Prefetch failures are silent. Opening the region retries the request and reports
a failure.

## Set up the runtime

`webui build` writes `lazy.webui.js` next to its other generated modules (the
`--modules-out` directory, which defaults to `--out`). Import it once, before
starting the renderer, instead of importing `@microsoft/webui-framework/lazy.js`
directly:

```typescript
import { startRuntime } from '@microsoft/webui-framework/runtime.js';
import '@microsoft/webui-framework/styles.js';
import '../dist/lazy.webui.js';
import './app-shell.js';

startRuntime();
```

The generated module imports the lazy runtime and registers, for each region,
a dynamic `import()` of every scripted component that region introduces:

```javascript
// Generated by WebUI. Do not edit.
import { defineLazyScripts } from '@microsoft/webui-framework/lazy.js';

defineLazyScripts({
  "app-shell::lazy:@0": {
    "settings-panel": () => import("../src/settings-panel/settings-panel.ts"),
  },
});
```

Your bundler compiles those imports like any other, so the components' code is
split into chunks, loaded from your bundler's public path, fingerprinted, and
listed in its manifests. Do not import those components statically: a region
loads its pack and its components' scripts in parallel, and renders its body
only after both have arrived, so the custom elements are defined before they
are inserted. In development builds the runtime warns when a component that only
lazy regions use is already defined at startup, which means a static import
pulled it into your startup bundle, and when a region's components have no
loader, which means the page imported `lazy.js` instead of `lazy.webui.js`.

A component's script is the `.ts` or `.js` file next to its `.html`. For a
component whose module lives elsewhere, such as in a package without sibling
scripts, map the tag to a specifier with `--component-module tag=specifier` (or
`componentModules` in `build()`). The build warns about a scripted component
that a region introduces without a known module.

When you build with a [state projection](../build/hydration#build-time-state-projection)
manifest, run `webui build` once without it to write the generated module,
bundle the client, then build again with the manifest. The generated module
never depends on the projection.

Pages without lazy regions do not need any of this and pay nothing for the
feature: the rendering core contains no lazy code. The import also adds
`$rendered()` and `$prefetch()` to your components; the build writes
`lazy.webui.d.ts` next to the module so TypeScript sees them without `allowJs`.
Activating a lazy region without the import throws an error that names it.

## Data

Lazy regions never fetch application state. Load data the way you normally do
and assign it to the component. If you set the condition before awaiting your
request, the body's pack downloads at the same time.

## Dialogs, popovers, and focus

Setting the condition is enough to show the body. Await `this.$rendered()` only
when code needs the body's DOM right away. The common case is a native dialog:
`showModal()` moves focus into the dialog immediately, so the body must exist
before you call it.

```html
<dialog w-ref="{settingsDialog}" @close="{closeSettings()}">
  <if condition="settingsOpen" lazy>
    <h2>Settings</h2>
    <input autofocus value="{{displayName}}">
  </if>
</dialog>
```

```typescript
async openSettings(): Promise<void> {
  this.settingsOpen = true;
  await this.$rendered();
  this.settingsDialog.showModal();
}

closeSettings(): void {
  this.settingsOpen = false;
}
```

`$rendered()` applies queued updates, waits for every active lazy region of the
component, including nested regions activated by that render and the scripts of
the components they introduce, and then resolves. It rejects if a pack or script
cannot be loaded, including a pack request that does not finish within 10
seconds. Calling it again, or setting the condition again, retries. If the
condition turns false before the pack arrives, the body is not rendered.

Refs inside a lazy body are valid while that view exists. Removing it clears
refs that still point to its nodes, so the owner does not keep a detached UI tree
alive. Deferring an inline body does not split its owner's JavaScript.

## Handle failures

When a region cannot open because its pack or a component script failed to
load, the owning component dispatches a `lazyerror` event. It bubbles, crosses
shadow roots, and carries the error in `detail.error`. Cancel it to handle the
failure yourself, for example to log it and reset the state that opened the
region; otherwise the error is passed to `reportError()`, so it reaches the
page's error reporting without an unhandled promise rejection.

```typescript
constructor() {
  super();
  this.addEventListener('lazyerror', event => {
    event.preventDefault();
    telemetry.log('settings-unavailable', event.detail.error);
    this.settingsOpen = false;
  });
}
```

Setting the condition again, or awaiting `$rendered()`, retries the load.

## Initial server rendering

The condition still controls SSR:

| Initial condition | Result |
| --- | --- |
| False | No body markup or body template is sent. The owner's template references the pack by its path. |
| True | The body renders with the initial state, its templates are inline, and the browser hydrates that DOM without fetching the pack. The scripts of the components it introduces load during hydration. |

Components used elsewhere on the page are not removed from it merely because one
use is lazy; when a pack also contains them, the runtime skips templates the page
already holds. State projection is not a secrecy boundary.

A native dialog's content can be server-rendered, but modal top-layer placement
and focus still use the browser's `showModal()` API.

## Styles

A region's styles follow the same `--dom` and `--css` rules as the rest of the
application, and its pack carries them like a route partial does:

- `--dom` decides which CSS tree owns a component's styles. A Light DOM
  component's styles join the tree that contains it once, however many instances
  render. Every Shadow DOM instance is its own tree, so the styles are installed
  into each shadow root. In Light DOM builds, the styles of Light components that
  a lazy region introduces are still delivered with their owner at startup;
  their templates and scripts are deferred. Shadow DOM components defer all
  three.
- `--css` decides the form. In `style` and `module` builds the pack carries the
  CSS text. In `link` builds the pack names the stylesheet files, and the
  template that holds the region's condition lists the same paths, so the
  browser requests them in parallel with the pack, at the same priority. Either
  way a region opens after one round trip.

## Memory

Once a region whose pack was loaded has been shown and no instance of it is
rendered anymore, the framework releases its body template at the next idle
period and clears its inventory bit. Regions that were only prefetched stay
loaded until they are first shown, and idle or eager prefetch does not load a
released region again. If a released region opens again before the browser
reclaims its template, it is reused without a request; otherwise its pack is
fetched again, normally from the HTTP cache. Component templates delivered by a
pack stay registered like other templates; use `collectTemplates()` from
`@microsoft/webui-framework/template-cache.js` to release those.

## Build and host packs

`webui build` writes packs to `<out>/_webui/packs/<owner>.<hash>.json`, and
`webui serve` serves them during development. Serve them as static files
beside the page:

- File names change whenever their content changes, so serve them with long-lived
  immutable caching, such as `Cache-Control: public, max-age=31536000, immutable`.
- Keep the previous build's packs available during a deployment window, as you
  would JavaScript chunks; a page always requests the packs of its own build.
- Pack paths are relative to the asset base, which maps to the build output and
  defaults to the document's base URL. Pass `--asset-base https://cdn.example.com/app/`
  (or `assetBase` in `build()`) to publish the output elsewhere, or set
  `$webui.assetBase` in the render state when the base is only known per
  request. Cross-origin packs need CORS. Document responses also carry the base
  in `<meta name="webui-asset-base">`, so the first lines of your entry can point
  your bundler's runtime public path at the same host before `startRuntime()`.
  That steers chunks your bundler's runtime loads; with native ES module output,
  chunks load relative to the entry script, so serve the entry from the same
  base.

Packs contain only compiled templates, condition functions, styles, and
inventory data. Hosts render pages with the usual APIs; there is no lazy-specific
host API. When you build programmatically, publish the returned pack files and
write the returned loader module where your bundler imports it:
`lazyPackFiles` and `lazyScriptsModule` from [Node](../build/integrations/node)
`build()`, `lazy_pack_files` and `lazy_scripts_module` from the
[Rust](../build/integrations/rust) `build()` (or call `webui::write_lazy_packs`),
and the same fields from `buildApp()` in the [WASM](../build/integrations/wasm)
build. Each pack entry also names its region, owner, trigger, size, the
component scripts it loads, and, in link builds, its stylesheets. `--metafile`
writes that report as `lazyPacks`, with brotli-compressed sizes, so your
deployment can upload and precache everything a region loads, for example in a
service worker, and CI can budget each region.

Condition functions in a pack run with the page's CSP nonce and the `webui`
Trusted Types policy, as in SSR. Allow the policy name `webui` when you enforce
Trusted Types.

Build with the WebUI plugin and matching compiler and runtime versions.

## Lazy regions and component assets

Use a lazy region for on-demand UI that a template owns: its body keeps the
owner's state, bindings, events, and refs, and its templates are freed after it
closes. Use [component assets](./interactivity#loading-static-component-assets)
for UI that application code creates outside templates with `create()`.
Component assets cannot contain lazy regions.

## When not to use it

The lazy runtime adds about 2.3 KB (brotli) to an application that imports it,
and each pack costs a request. A small body that is cheap to deliver with the
page, or UI that is used immediately, is usually better as an ordinary `<if>`.
Lazy regions pay off when an application has many bodies that are not initially
visible, or a few large ones. Their trade-offs are measured in the
[lazy UI benchmark](https://github.com/microsoft/webui/tree/main/examples/integration/lazy-ui-bench).
