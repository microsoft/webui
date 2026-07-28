# Streaming Boundaries

`<webui-boundary>` marks a complete part of an entry page that can be flushed
and hydrated before the rest of the response arrives. It is available with the
WebUI parser plugin and the opt-in Rust streaming render path.

```html
<head>
  <script type="module" async src="/index.js"></script>
</head>
<body>
  <webui-boundary name="composer">
    <message-composer></message-composer>
  </webui-boundary>

  <webui-boundary name="feed">
    <activity-feed></activity-feed>
  </webui-boundary>
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
  option, but each stylesheet is a separate request. Under real latency that
  leaves the critical island unstyled for a full round trip even though
  WebUI emits `<link rel="preload">` in `<head>`. Prefer it when repeat visits
  matter more than first-visit paint, or when boundaries are low priority.

::: warning Serve the generated stylesheets
`link` and `module` reference files that only exist in the build result, not in
your client bundler's output directory. Serve every entry of
`BuildResult::css_files` or the stylesheets 404 and the page renders unstyled.
The streaming example does this in
`server/src/assets.rs::insert_generated_css`.
:::

WebUI applies one strategy per build, so a page whose critical boundary wants
`style` and whose repeated feed wants `link` must currently pick one. Choose for
the highest-priority boundary.

See [Hydration](/guide/concepts/hydration#progressive-streaming-hydration) for
loading, lifecycle, CSP, and prioritization guidance.
