# Streaming Boundaries

`<boundary>` splits an entry page into complete regions that WebUI can flush
and hydrate before the full response arrives.

A boundary is a compile-time directive, not a DOM element. WebUI removes it
from the rendered HTML and streams its children in normal document order.

## 1. Author the checkpoints

Put boundaries around independently useful page regions, ordered by priority:

```html
<head>
  <script type="module" async src="/index.js"></script>
</head>
<body>
  <boundary name="weather-shell">
    <weather-panel status="loading"></weather-panel>
  </boundary>

  <boundary name="composer-ready">
    <message-composer></message-composer>
  </boundary>

  <boundary name="feed">
    <activity-feed></activity-feed>
  </boundary>
</body>
```

Import the streaming coordinator before component registration modules:

```typescript
import '@microsoft/webui-framework/streaming.js';
import './weather-panel/weather-panel.js';
import './message-composer/message-composer.js';
import './activity-feed/activity-feed.js';
```

The application module must be `async` and appear in `<head>` before boundary
content so early checkpoints can hydrate while the document is still parsing.

## 2. Choose how the server drives the response

| Need | Use |
|---|---|
| Render all boundaries immediately with one state value | Rust `WebUIHandler::render_streaming` |
| Control when each boundary commits or send later state | Rust `WebUIHandler::stream_response` |
| Let an API backend control readiness while `webui serve` owns rendering | `webui serve --api-port` |
| Stream directly from Node, WASM, .NET, or C | That handler's streaming session API |

All paths produce the same ordered browser protocol.

## 3. Drive a host-controlled response

Resolve authored names once to integer boundary handles. Then use these four
operations:

| Operation | Purpose |
|---|---|
| `write_shell(state)` | Flush everything before the first boundary |
| `write_boundary(id, state, mode)` | Render and flush the next boundary |
| `update(id, state)` | Patch an earlier boundary committed as `Updatable` |
| `finish(state)` | Render the tail, emit the terminal record, and end the response |

The required order is:

```text
write_shell -> write_boundary* -> finish
```

`update` may run between boundary writes, but only after its target has
committed as updatable.

```rust
use webui::{BoundaryMode, RenderOptions, WebUIHandler};

let options = RenderOptions::new("index.html", "/");
let mut response =
    handler.stream_response(&protocol, &options, &mut writer)?;

let weather = response.boundary("weather-shell")?;
let composer = response.boundary("composer-ready")?;
let feed = response.boundary("feed")?;

response.write_shell(&page_state)?;
response.write_boundary(
    weather,
    &loading_weather,
    BoundaryMode::Updatable,
)?;
response.write_boundary(
    composer,
    &composer_state,
    BoundaryMode::Final,
)?;

response.update(weather, &ready_weather)?;
response.write_boundary(feed, &feed_state, BoundaryMode::Final)?;
response.finish(&tail_state)?;
```

Boundary HTML always commits once in declaration order. Backend work may run
concurrently, but a later boundary cannot overtake an earlier one.

### Final or updatable?

| Mode | Use it when | Browser retention |
|---|---|---|
| `Final` | The boundary needs no later server state | Releases boundary roots after hydration |
| `Updatable` | A complete shell should hydrate now and receive state later | Retains only successfully activated roots until `finish` |

Use `Final` by default. An update calls the component's normal `setState()`
path and never re-runs hydration or `hydratedCallback()`. If the component
module is still loading, WebUI hydrates the server-rendered DOM first, then
replays the latest queued patch through `setState()`.

## 4. Drive streaming through `webui serve`

With `webui serve --api-port`, the API backend can return newline-delimited
control records:

```text
{"type":"shell","version":1,"state":{"feed":[]}}
{"type":"boundary","name":"weather-shell","mode":"updatable"}
{"type":"boundary","name":"composer-ready"}
{"type":"update","name":"weather-shell","state":{"status":"ready"}}
{"type":"boundary","name":"feed"}
{"type":"finish"}
```

These records go from the backend to the CLI, not to the browser. The CLI
resolves names, renders the compiled template in Rust, and streams the resulting
HTML. See [`webui serve --api-port`](/guide/cli/) for limits and fallback
behavior.

## Authoring rules

- `name` is required, static, non-empty, and unique in the entry template.
- Author boundaries only in the outermost entry template.
- Boundaries cannot nest or appear inside `<if>`, `<for>`, or `<route>`.
  They may wrap a complete directive scope.
- Do not place a boundary inside registered component host content, raw or
  inert elements such as `<script>` or `<template>`, or table/select parser
  contexts. Wrap the complete host, element, or table instead.
- Every registered WebUI component rendered in streaming mode must be inside
  an explicit boundary. Native static HTML may remain outside.
- Never author `<webui-hydrate>`; it is generated runtime output.

Invalid placement fails the build with an actionable diagnostic.

## Production checklist

- Pass the generated `webui-projection.json` to `BuildOptions` in custom Rust
  builds. Without it, every checkpoint falls back to serializing full state.
- Preserve HTTP backpressure and cap concurrent streaming renders.
- Disable reverse-proxy response buffering for the streaming route, for example
  with `X-Accel-Buffering: no` in nginx.
- Use an updatable boundary for slow data instead of blocking later
  checkpoints.
- Use `examples/app/streaming` as the complete reference application.

## More detail

- [Rust streaming integration](/guide/integrations/rust#streaming-ssr)
- [Node streaming sessions](/guide/integrations/node#progressive-streaming)
- [WASM streaming sessions](/guide/integrations/wasm#streamingsession)
- [C and FFI streaming sessions](/guide/integrations/ffi)
- [Hydration lifecycle and diagnostics](/guide/concepts/hydration#progressive-streaming-hydration)
- [Performance model](/guide/concepts/performance)
