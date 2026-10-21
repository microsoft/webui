# Lazy template bodies

Try the [popover and dialog example](https://github.com/microsoft/webui/tree/main/examples/app/lazy-ui):
`cargo xtask dev lazy-ui`. Its `lazy-popover` and `lazy-dialog`
components own native overlays, with all interior content in an inline lazy
body. Loading and retry feedback stay at the trigger until the overlay is ready.

Add `lazy` to an ordinary `<if>` when its UI should stay compiled but not be
delivered until it is needed. The body can contain native HTML, multiple roots,
conditions, repeats, refs, events, and component calls. It does not become a
component: state, handlers, refs, and loop variables keep their existing owner.
As with other reactive directives, WebUI browser activation runs inside an
existing component template; the lazy body does not introduce another one.

```html
<!-- app-shell.html -->
<button @pointerenter="{preloadSettings()}" @focus="{preloadSettings()}"
  @click="{openSettings()}">Settings</button>

<dialog w-ref="{settingsDialog}" @close="{onSettingsClosed()}">
  <button @click="{dismissSettings()}">Close</button>
  <if condition="settingsLoading"><p role="status">Loading...</p></if>
  <if condition="settingsError"><p role="alert">{{settingsError}}</p></if>

  <if condition="settingsReady" lazy id="settings">
    <h2>Settings</h2>
    <p>{{displayName}}</p>
    <button @click="{saveSettings()}">Save</button>
  </if>
</dialog>
```

`lazy` is boolean. The old `<if defer>` spelling is rejected rather than aliased.
Use the matching `lazy.js` imports and `renderLazy*` host APIs, and rebuild both
protocol and client; the former names and `/_webui/deferred` endpoints are not
supported. Native `<script defer>` is unrelated and unchanged.

This is different from `w-hydrate="lazy"` and `w-render="lazy"`, which postpone
work on already server-rendered components. `<if lazy>` omits an inactive
template body's markup and metadata from initial delivery.

Use an optional `id` for application-controlled loading;
names are unique within the owning template and may contain letters, digits,
periods, underscores, and hyphens, up to 128 characters. The directive does not
produce an HTML element or DOM id. Unnamed regions have compiler-managed
identities; do not hardcode them.

## Initial server rendering

The condition still controls SSR:

| Initial condition | Result |
| --- | --- |
| False | No body markup or lazy-only template metadata is sent. |
| True | The body renders with the initial request's state and required metadata. The browser hydrates that DOM without a second state request or replacement host. |

Existing CSS ownership and DOM-mode rules are unchanged. Owner styles and
dependencies used elsewhere are not removed merely because one use is lazy.
Likewise, deferral does not remove unrelated state the host supplied to an
active owner; state projection is not a secrecy boundary.

A native dialog's content can be server-rendered, but modal top-layer placement
and focus still use the browser's `showModal()` API.

## Select the renderer and delivery capabilities

The WebUI renderer adapter contains no fetching or routing:

```typescript
import '@microsoft/webui-framework/lazy.js';
```

Inventory, preloading, transport, and instance-independent resource reuse remain
in the hydration-agnostic client package. Importing lazy delivery does not
start routing or install a rendering engine:

```typescript
import { createLazyLoader } from '@microsoft/webui-router/lazy.js';
import { lazyNdjson } from '@microsoft/webui-router/lazy-ndjson.js';

const fragments = createLazyLoader({
  endpoint: '/_webui/lazy',
  transport: lazyNdjson,
});
```

Omit `transport` for JSON-only delivery. That composition does not include the
NDJSON reader. `lazyNdjson` accepts JSON fallback and limits each NDJSON
record to 256 KiB; use `createLazyNdjson(maxRecordBytes)` for a different
finite record budget. `timeoutMs` defaults to 10 seconds and covers the complete
request, including state and resource preparation.

## Preload resources, then request fresh state

Inside the owning `app-shell` class:

```typescript
private settingsRequest?: AbortController;
private settingsGeneration = 0;

async preloadSettings(): Promise<void> {
  try {
    await fragments.ensureLoaded('app-shell', 'settings');
  } catch (error) {
    this.settingsError = String(error);
  }
}

async openSettings(): Promise<void> {
  const generation = ++this.settingsGeneration;
  this.settingsRequest?.abort();
  const request = new AbortController();
  this.settingsRequest = request;
  this.settingsLoading = true;
  this.settingsError = '';
  if (!this.settingsDialog.open) this.settingsDialog.showModal();

  try {
    const state = await fragments.fetchPartial('app-shell', 'settings', {
      url: '/ui/settings',
      signal: request.signal,
    });
    if (generation !== this.settingsGeneration || request.signal.aborted) return;
    this.setState(state);
    this.settingsReady = true;
  } catch (error) {
    if (generation === this.settingsGeneration && !request.signal.aborted) {
      this.settingsError = String(error);
    }
  } finally {
    if (generation === this.settingsGeneration) this.settingsLoading = false;
  }
}

dismissSettings(): void {
  this.settingsDialog.close();
}

onSettingsClosed(): void {
  this.settingsGeneration++;
  this.settingsRequest?.abort();
  this.settingsRequest = undefined;
  this.settingsReady = false;
  this.settingsLoading = false;
  this.displayName = '';
}
```

Declare the mutable template fields as observables, as with ordinary
[interactive components](./interactivity). The body mounts after state and
resources are ready; it has no separate component lifecycle. `fetchPartial()`
returns state and never applies it globally or mounts DOM. When a body is inside
a repeat, update the appropriate owner/collection state before activating it.

`ensureLoaded()` fetches no application state and creates no UI. Reopening still
requests fresh state even when all resources are available. A state request does
not wait for an unfinished speculative preload. Concurrent instances share
resources, not state or cancellation.

Refs inside a lazy body are valid while that view exists. Removing it clears
refs that still point to its nodes, so the owner does not keep a detached UI tree
alive. A newer ref assignment is not cleared by an older view's cleanup.

Optional behavior modules remain explicit. Deferring an inline body does not
split its owner's JavaScript automatically:

```typescript
import { createLazyLoader, lazyFragment } from '@microsoft/webui-router/lazy.js';

const fragments = createLazyLoader({
  loaders: {
    [lazyFragment('app-shell', 'settings')]: () => import('./settings-logic.js'),
    'custom-input': () => import('./custom-input.js'),
  },
});
```

Handlers in the lazy body still resolve on the existing owner. Optional
modules must provide the behavior that the application expects; the loader does
not turn a fragment into a custom element.

## Host endpoints

Use the same loaded protocol for SSR and lazy requests. Hosts authenticate
and authorize before committing headers, and own data loading, cancellation,
backpressure, and flushing.

| Operation | Purpose |
| --- | --- |
| `renderLazyResources(owner, id, options?)` | Resource-only JSON for preload |
| `prepareLazyResources(owner, id, options?)` | Resource JSON plus its acknowledgement for two-phase delivery |
| `renderLazyPartial(state, owner, id, options?)` | Complete JSON, or the final NDJSON state record |

The options are `inventory`, `styleInventory`, and `buildId`, supplied by the
client's corresponding `X-WebUI-*` headers. For NDJSON, flush the prepared
resource record, await the application's state work, then render the state
record with `prepared.options`. This avoids resending the first record's
resources or reparsing its metadata. A final state record can add resources for
nested lazy bodies that the returned state activates.

See the [Node](../integrations/node#lazy-template-responses),
[Rust](../integrations/rust#lazy-template-responses),
[Python](../integrations/python#lazy-template-responses),
[.NET](../integrations/dotnet#lazy-template-responses),
[WASM](../integrations/wasm#lazy-template-responses), and
[C](../integrations/ffi#lazy-template-responses) APIs.

For local development, `webui serve` provides `/_webui/lazy?owner=...&id=...`
and `/_webui/lazy/state?owner=...&id=...`. The state endpoint uses configured
state or the API state provider and supports JSON/NDJSON negotiation. Data
failures are explicit, including failures after metadata has been sent.

Build with the WebUI plugin and the matching compiler/runtime. This
protocol-backed feature is not combined with `--emit-component-assets`.
Inventory belongs to one artifact identity; a mismatch requires a document
refresh rather than reinterpreting old bits. Explicit collection invalidates
availability, and an in-flight request retries against fresh inventory before
returning state.
