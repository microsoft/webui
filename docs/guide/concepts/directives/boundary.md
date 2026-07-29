# Streaming Boundaries

`<boundary>` marks a complete part of an entry page that can be flushed
and hydrated before the rest of the response arrives. It is available with the
WebUI parser plugin and the opt-in Rust streaming render path.

```html
<head>
  <script type="module" async src="/index.js"></script>
</head>
<body>
  <boundary name="composer">
    <message-composer></message-composer>
  </boundary>

  <boundary name="feed">
    <activity-feed></activity-feed>
  </boundary>
</body>
```

The directive is removed at compile time. It does not add an element to the
application DOM. A normal `WebUIHandler::render` call renders its children as
usual without enabling progressive hydration. Use
`WebUIHandler::render_streaming` with a `FlushWriter` to commit its checkpoints.

The async browser entry must install the streaming coordinator before importing
component registration modules:

```typescript
import '@microsoft/webui-framework/streaming.js';
import './message-composer/message-composer.js';
import './activity-feed/activity-feed.js';
```

The normal `@microsoft/webui-framework` entry deliberately excludes coordinator
code.

## Authoring Rules

In the current Phase 1 implementation:

- `name` is required, non-empty, static, and unique within the entry template.
  It cannot contain a <code v-pre>{{binding}}</code>.
- A boundary can appear only in the outermost entry template. Boundaries in
  reusable components and route-shell component templates are not supported.
- A boundary cannot appear inside another boundary, `<if>`, `<for>`, or
  `<route>`. Put it in the entry template so it fully wraps any such scope.
- A boundary cannot appear inside `<table>`, `<thead>`, `<tbody>`, `<tfoot>`,
  `<tr>`, `<colgroup>`, `<select>`, or `<optgroup>`. In those contexts the
  browser's HTML parser foster-parents the generated `<webui-hydrate>` sentinel
  out of the table while the payload script stays inside, which permanently
  breaks hydration. Wrap the whole table in a boundary instead, or place the
  boundary inside a `<td>`, `<th>`, or `<caption>` — those return to normal
  insertion rules and are allowed.
- During `render_streaming`, every registered WebUI component host must be
  inside an explicit boundary. The handler rejects an outside host because it
  cannot safely activate that component after parser-time upgrade. Native HTML
  and unregistered static tail markup may remain outside boundaries.
- `<webui-hydrate>` is reserved for generated runtime output and must never be
  authored.

Invalid markup fails the build with a structured diagnostic such as
`missing-boundary-name`, `invalid-boundary-name`,
`duplicate-boundary-name`, `nested-boundary`, `boundary-crosses-scope`,
`boundary-in-foster-context`, or `authored-webui-hydrate`.

## Send Boundary-Local State

Pass the client build's `webui-projection.json` to `BuildOptions` whenever an
entry declares boundaries:

```rust
let mut options = BuildOptions::new(app_root, "index.html");
options.plugin = Some(Plugin::WebUI);
options.projection_manifests = vec![app_root.join("dist/webui-projection.json").into()];
```

Without a manifest the build falls back to full-state hydration, so **every**
checkpoint serializes the entire application state instead of just its own
components' keys. That turns the wire cost into `O(boundaries × full state)`.
The build reports this as a `streaming-without-projection` warning.

On the streaming example this single option cut the response from 19,403 to
12,228 bytes, and steady-state feed checkpoints from 1,470 bytes to 35.

Boundaries are committed only in document order. They do not start asynchronous
server work, move later content ahead of earlier content, or replace an earlier
skeleton with later same-response HTML.

### Slow surfaces: place the skeleton in its own boundary

Because of that ordering rule, a region that depends on slow backend data must
not stall the response — doing so delays every boundary after it, including the
critical one. Give the slow surface a *complete* placeholder component inside
its own early boundary instead, then let it resolve its own data after it
hydrates:

```html
<header>
  <boundary name="weather-shell">
    <weather-panel status="loading"></weather-panel>
  </boundary>
</header>
```

```typescript
connectedCallback(): void {
  super.connectedCallback();
  void this.loadForecast(); // Runs once this boundary commits.
}
```

The boundary carries no server data, so it commits immediately and the next
boundary follows in the same flush window. `examples/app/streaming` ships this
exact pattern.

### Load an island's code with the island

A boundary can carry its own `<script>`. Put the island's module inside it and
the browser fetches that code when the chunk reaches the parser, so your
critical entry never carries bytes the first interaction does not need:

```html
<boundary name="weather-shell">
  <script type="module" async src="./weather-panel.js"></script>
  <weather-panel status="loading"></weather-panel>
</boundary>
```

This is safe by construction. The boundary commits before the class exists, so
the coordinator stashes that boundary's state on the root and waits on
`customElements.whenDefined()`, activating it the moment the module registers.
`webui:hydration-complete` stays open until then, and the script is authored
content, so boundary teardown leaves it alone.

::: warning Splitting an island is not yet a net win
Splitting an island into its own bundle entry makes your bundler hoist the
framework runtime into a chunk your critical entry statically imports. The
preload scanner cannot see that chunk behind your `<script>` tag, so the
browser only discovers it after downloading and parsing the entry — a full
round trip on the critical path.

That round trip cancels out what splitting saves. On `examples/app/streaming`
over a throttled link, splitting alone was a wash: 1074 ms composer
time-to-interactive bundled versus 1061 ms split. Adding
`<link rel="modulepreload">` for the shared chunk recovers it — 956 ms, a
10.9% win — but only in the right order. Preloads are issued in document order
and share the connection, so listing a 284-byte chunk ahead of a 35 KB one
delays the long pole behind it and gives the whole win back (1076 ms).

WebUI does not emit those hints yet, and you cannot write them yourself because
your bundler content-hashes the filenames. Until it does, split an island out
for the *architectural* benefit — code that loads with its boundary rather than
ahead of the first interaction — and do not expect a time-to-interactive win.
For a sub-kilobyte component the extra request is not worth it either way.
:::

Each checkpoint includes projected state and first-use metadata for the
component graph reachable from roots rendered in that boundary. This includes
initially hidden conditional/repeat descendants so they can appear after a
client state change without a page-global bootstrap. Inventory remains limited
to roots that actually rendered, and unrelated later boundaries remain
excluded.

## Pick a CSS Strategy for Streaming

Component CSS is delivered by [`BuildOptions.css`](/guide/integrations/rust), and the
choice matters more when streaming than it does for a buffered response.
`style` inlines each component's stylesheet into **every** rendered shadow root,
so a repeated component pays for its CSS once per instance. A four-item feed
already emits `feed-item`'s rules five times — four instances plus the template
metadata.

Measured on the streaming example (two components, four feed items), with a
cold browser context, six runs, 100 ms RTT and 1.6 Mbps down:

| Strategy | Response | Composer styled | Delivery |
| --- | --- | --- | --- |
| `style` | 12,228 B | 147 ms | Inline `<style>`, repeated per instance |
| `module` | 10,061 B (−17.8%) | 158 ms (+11 ms) | One `data:` import map per component |
| `link` | 8,368 B (−31.6%) | 268 ms (+121 ms) | One cacheable request per component |

Time to interactive was unchanged (615–617 ms) — it is gated by the application
bundle, not by CSS. What changes is when the critical island stops being
unstyled.

- **`style`** costs the most bytes but is the only strategy with **zero extra
  round trips**: a boundary's CSS arrives in the same chunk as its markup. Use
  it when a critical boundary must paint styled immediately.
- **`module`** removes per-instance duplication while keeping CSS in the
  response, so it recovers most of the byte savings for only ~11 ms. This is
  usually the best default for pages that stream many repeated components.
- **`link`** is the smallest response and the only cacheable-across-navigations
  option, but each stylesheet is a separate request. WebUI emits
  `<link rel="preload">` in `<head>` so that request starts as early as
  possible; under real latency it still costs a round trip before the shadow
  root is styled. Prefer it when repeat visits matter more than first-visit
  paint, or when the boundaries involved are low priority.

::: warning Serve the generated stylesheets
`link` and `module` reference component stylesheets, which the WebUI build
returns in `BuildResult::css_files` rather than writing to your client
bundler's output directory. `webui build` writes them to disk and `webui serve`
serves them for you, but a **custom server must serve them itself** — otherwise
the markup and preload hints are correct while every stylesheet URL 404s and
the page renders unstyled. See
`examples/app/streaming/server/src/assets.rs::insert_generated_css`.
:::

WebUI applies one strategy per build, so a page whose critical boundary wants
`style` and whose repeated feed wants `link` must currently pick one. Choose for
the highest-priority boundary.

See [Hydration](/guide/concepts/hydration#progressive-streaming-hydration) for
loading, lifecycle, CSP, and prioritization guidance.
