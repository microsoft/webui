# Streaming Boundaries

`<boundary>` marks a complete region that WebUI can render, flush, and hydrate
before the rest of the response arrives. It is a compile-time directive and
does not create a DOM wrapper.

## Put boundaries where readiness changes

An entry can contain one component while that component owns the useful
checkpoint:

```html
<!-- index.html -->
<html>
  <head>
    <script type="module" async src="/index.js"></script>
  </head>
  <body>
    <ntp-page></ntp-page>
  </body>
</html>
```

```html
<!-- ntp-page.html -->
<main>
  <h1>{{title}}</h1>

  <boundary name="search-ready">
    <search-box query="{{query}}"></search-box>
  </boundary>

  <section class="feed">{{slowFeed}}</section>
</main>
```

The server discovers `search-ready` while rendering `<ntp-page>`. It can commit
and hydrate `<search-box>` before the remaining parent content arrives. WebUI
creates the required parent span automatically. Do not add an outer boundary
around `<ntp-page>`.

Load the coordinator before component registrations:

```typescript
import '@microsoft/webui-framework/streaming.js';
import './ntp-page.js';
import './search-box.js';
```

The application entry must load early with `async`, or an equivalent
non-blocking strategy, in `<head>`.

## Runtime occurrences

A declaration becomes an occurrence only when rendering reaches it. Boundaries
are allowed in:

- entry templates
- reusable component templates
- true `<if>` branches
- each `<for>` iteration
- selected route content and outlets

False conditions, empty loops, and unselected routes produce no occurrence.
The host receives the next occurrence as:

```text
{ instanceId, declarationId, owner, name, key }
```

- `instanceId` identifies this occurrence in one response.
- `declarationId` identifies the compiled declaration.
- `owner` is the entry or component template that authored it.
- `name` is unique only within that owner.
- `key` identifies a repeated occurrence.

Use `owner`, `name`, and `key` to decide what state to load. Pass `instanceId`
back to the session.

### Repeated boundaries need keys

```html
<for each="result in results">
  <boundary name="result-actions" key="{{result.id}}">
    <result-actions result-id="{{result.id}}"></result-actions>
  </boundary>
</for>
```

A repeated declaration must have a key. At runtime the key must resolve to a
string or finite JSON number, and live occurrences of that declaration must
have unique keys. This also applies when a component containing a boundary is
rendered more than once.

## Drive the response

Every host binding exposes the same three operations:

| Operation | Result |
|---|---|
| `start(state)` | Bytes through the first occurrence, or a completed step |
| `resume(instanceId, state, mode)` | Commit that pending occurrence and continue |
| `update(instanceId, patch)` | State-only bytes for a committed updatable occurrence |

`start` and `resume` return bytes, `done`, and the next optional descriptor.
When `done` is true, those bytes already include the parent tail, terminal
record, and document close.

```rust
let mut response =
    handler.stream_response(&protocol, &options, &mut writer)?;
let mut step = response.start(&initial_state)?;

while !step.done {
    let boundary = step.boundary.as_ref().expect("pending descriptor");
    let state = load_state(&boundary.owner, &boundary.name, boundary.key.as_ref());
    step = response.resume(
        boundary.instance_id,
        &state,
        BoundaryMode::Final,
    )?;
}
```

The example uses `expect` only for brevity. Production code should return an
error if an unfinished step has no descriptor.

### Final or updatable

| Mode | Use when |
|---|---|
| `Final` | No later server state is needed |
| `Updatable` | Complete HTML should hydrate now and accept state later |

An update is a shallow projected state patch. It uses the component's normal
reactive `setState()` path. It does not insert markup, replace DOM, or rerun
hydration or `hydratedCallback()`.

## State at a suspension

WebUI freezes only the projected parent keys needed to continue, plus lexical
locals such as the current loop item and component attributes. Resume state
overlays that frozen parent state. Resolution order remains:

1. lexical locals
2. state supplied to `resume`
3. frozen parent state

This lets a loop body keep `item` while a host supplies fresh boundary data.

## Authoring rules

- `name` is required, static, non-empty, and unique within its owner.
- Authored boundaries cannot contain another boundary, directly or through a
  component or runtime branch.
- A repeated declaration requires `key`.
- Do not place a boundary in component host children, raw or inert elements,
  authored `<template>`, or table/select foster-parenting contexts.
- Never author `<webui-hydrate>` or the `data-ws*` attributes. WebUI owns them.

Invalid authoring fails the build with a stable diagnostic and actionable help.

## Production checklist

- Preserve HTTP backpressure and cap concurrent rendering sessions.
- Disable response buffering in proxies and CDNs where appropriate.
- Pass the request CSP nonce so generated record scripts are allowed.
- Supply a projection manifest to keep checkpoint state local and small.
- Use `Final` unless later state is required.

See [Hydration](/guide/concepts/hydration#progressive-streaming-hydration),
[Performance](/guide/concepts/performance), and the
[integration guides](/guide/integrations/) for host-specific examples.
