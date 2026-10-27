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
<button @click="{openSettings()}">Settings</button>

<dialog w-ref="{settingsDialog}" @close="{onSettingsClosed()}">
  <if condition="settingsReady" lazy>
    <h2>Settings</h2>
    <p>{{displayName}}</p>
    <button @click="{saveSettings()}">Save</button>
  </if>
</dialog>
```

The build compiles each lazy body into a static, content-addressed **template
pack**. The page only references it. When the condition becomes true, the
framework fetches the pack, registers its templates, and renders the body. No
server endpoint, region id, or loader code is involved.

This is different from `w-hydrate="lazy"` and `w-render="lazy"`, which postpone
work on already server-rendered components. `<if lazy>` omits an inactive body's
markup and templates from the initial document.

## Choose when to prefetch

The optional value of `lazy` sets when the pack is fetched before the condition
becomes true. Prefetching only fetches and registers templates; it never renders
or requests application state.

| Declaration | Prefetches | Use for |
| --- | --- | --- |
| `lazy` or `lazy="intent"` | When a pointer moves over, focus enters, or a touch starts inside the owning component | Most dialogs, popovers, and panels behind a control |
| `lazy="idle"` | When the browser is idle after the page loads | UI that is likely to be used soon |
| `lazy="load"` | Right after startup hydration | UI that is needed almost immediately |
| `lazy="demand"` | Never; it loads when activated or prefetched | Rare UI, and regions nested in other lazy regions |

Idle and load prefetch are skipped when the user has enabled data saving. Call
`this.$prefetch()` to load all of a component's lazy regions on a signal of your
own, such as a keyboard shortcut or a route change.

Prefer `intent` for UI that opens from a control in the owner. A region that is
fetched only on activation costs at least one extra network round trip after the
click, even from the HTTP cache.

## Set up the runtime

Import the lazy runtime once, before starting the renderer:

```typescript
import { startRuntime } from '@microsoft/webui-framework/runtime.js';
import '@microsoft/webui-framework/styles.js';
import '@microsoft/webui-framework/lazy.js';
import './app-shell.js';

startRuntime();
```

Pages without lazy regions do not need this import and pay nothing for the
feature: the rendering core contains no lazy code. The import also adds
`$rendered()` and `$prefetch()` to your components, along with their TypeScript
types. Activating a lazy region without it throws an error that names the
import.

## Load state and open

State remains owned by your application. Fetch it the way you fetch any other
data, set the condition, and await `this.$rendered()` before you use anything
inside the body:

```typescript
private settingsRequest?: AbortController;

async openSettings(): Promise<void> {
  this.settingsRequest?.abort();
  const request = new AbortController();
  this.settingsRequest = request;
  try {
    // Fetch the template pack in parallel with the state.
    const [state] = await Promise.all([
      fetch('/api/settings', { signal: request.signal }).then(response => response.json()),
      this.$prefetch(),
    ]);
    if (request.signal.aborted) return;
    this.setState(state);
    this.settingsReady = true;
    await this.$rendered();
    this.settingsDialog.showModal();
  } catch (error) {
    if (!request.signal.aborted) this.settingsError = String(error);
  }
}

onSettingsClosed(): void {
  this.settingsRequest?.abort();
  this.settingsReady = false;
}
```

`$rendered()` applies queued updates, waits for every active lazy region of the
component, including nested regions activated by that render, and then resolves.
It rejects if a pack cannot be loaded, including a request that does not finish
within 10 seconds. Calling it again, or activating the region again, retries.
If the condition turns false before the pack arrives, the body is not rendered.

Refs inside a lazy body are valid while that view exists. Removing it clears
refs that still point to its nodes, so the owner does not keep a detached UI tree
alive. Deferring an inline body does not split its owner's JavaScript.

## Initial server rendering

The condition still controls SSR:

| Initial condition | Result |
| --- | --- |
| False | No body markup or body template is sent. The owner's template references the pack URL. |
| True | The body renders with the initial state, its templates are inline, and the browser hydrates that DOM without fetching the pack. |

Existing CSS ownership and DOM-mode rules are unchanged. Components used
elsewhere on the page are not removed from it merely because one use is lazy;
when a pack also contains them, the runtime skips templates the page already
holds. State projection is not a secrecy boundary.

A native dialog's content can be server-rendered, but modal top-layer placement
and focus still use the browser's `showModal()` API.

## Memory

Once a region whose pack was loaded has been shown and no instance of it is
rendered anymore, the framework releases its body template at the next idle
period and clears its inventory bit. Regions that were only prefetched stay
loaded until they are first shown, and idle or load prefetch does not load a
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
- Pack URLs are relative to the document base URL. Pass
  `--pack-public-base https://cdn.example.com/app/packs/` (or `packPublicBase`
  in `build()`) to serve them elsewhere. Cross-origin packs need CORS.

Packs contain only compiled templates, condition functions, styles, and
inventory data. Hosts render pages with the usual APIs; there is no lazy-specific
host API. When you build programmatically, publish the returned pack files:
`lazyPackFiles` from [Node](../integrations/node) `build()`,
`lazy_pack_files` from the [Rust](../integrations/rust) `build()` (or call
`webui::write_lazy_packs`), and `buildApp()` in the [WASM](../integrations/wasm)
build.

Condition functions in a pack run with the page's CSP nonce and the `webui`
Trusted Types policy, as in SSR. Allow the policy name `webui` when you enforce
Trusted Types.

Build with the WebUI plugin and matching compiler and runtime versions. This
feature is not combined with `--emit-component-assets`.

## When not to use it

The lazy runtime adds about 1.8 KB (brotli) to an application that imports it,
and each pack costs a request. A small body that is cheap to deliver with the
page, or UI that is used immediately, is usually better as an ordinary `<if>`.
Lazy regions pay off when an application has many bodies that are not initially
visible, or a few large ones. Their trade-offs are measured in the
[lazy UI benchmark](https://github.com/microsoft/webui/tree/main/examples/integration/lazy-ui-bench).
