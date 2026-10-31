---
title: "Streaming boundaries"
description: "Mark regions that can become interactive early, then pace their HTML and state through a host-owned streaming response."
---

# Streaming Boundaries

`<boundary>` marks a complete region that WebUI can render, flush, and hydrate
before the rest of the response arrives. It is a compile-time directive and
does not create a DOM wrapper. Use it with a host's progressive streaming API
and the browser streaming coordinator; adding the directive alone does not
turn an ordinary render into a progressively hydrated response.

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

The server reaches `search-ready` while rendering `<ntp-page>`. It can send
`<search-box>` for hydration before the remaining parent content arrives.
Do not add an outer boundary around `<ntp-page>`: boundaries cannot nest.
`resume` writes through the checkpoint; `advance` writes the following parent
content.

Load the coordinator before component registrations:

```typescript
import { startRuntime } from '@microsoft/webui-framework/runtime.js';
import '@microsoft/webui-framework/styles.js';
import '@microsoft/webui-framework/streaming.js';
import './ntp-page.js';
import './search-box.js';

startRuntime();
```

The coordinator must load early with `async`, or an equivalent non-blocking
strategy, in `<head>`. To keep the application entry separate, use the
[coordinator asset integration](/build/hydration#separate-coordinator-and-application-assets).
Component registrations may arrive later; a boundary becomes interactive only
after the registrations it needs are available.

## Runtime occurrences

A declaration becomes an occurrence only when rendering reaches it. Boundaries
are allowed in:

- entry templates
- reusable component templates
- true `<if>` branches
- selected route content and outlets

False conditions and unselected routes produce no occurrence. A boundary is
not allowed in a `<for>` body, directly or through a component, condition,
route, or outlet reached from that body. The build fails with
`boundary-in-repeat`. A `<for>` may be wholly inside one boundary, and
boundaries before or after a `<for>` are valid.

The host receives the next occurrence as:

```text
{ instanceId, declarationId, owner, name, key }
```

- `instanceId` identifies this occurrence in one response.
- `declarationId` identifies the compiled declaration.
- `owner` is the entry or component template that authored it.
- `name` is unique only within that owner.
- `key` distinguishes multiple static occurrences of one component-owned
  declaration when required.

Use `owner`, `name`, and `key` to decide what state to load. Pass `instanceId`
back to the session.

### Keys for multiple static callsites

```html
<!-- result-card.html -->
<boundary name="result-actions" key="{{resultId}}">
  <result-actions result-id="{{resultId}}"></result-actions>
</boundary>

<!-- index.html -->
<result-card result-id="first"></result-card>
<result-card result-id="second"></result-card>
```

When one entry traversal reaches a boundary-bearing component from more than one
static callsite, that component's declaration must have a key. Independent
entries that each call the component once do not trigger this rule. At runtime
the key must resolve to a string or finite JSON number, and simultaneously live
occurrences of that declaration must have unique keys. `<for>` is not a source
of multiple boundary occurrences because boundary-bearing subtrees under its
body are rejected.

## Drive the response

Every host binding exposes the same four operations:

| Operation | Result |
|---|---|
| `start(state)` | Bytes through the first occurrence, or a completed step |
| `resume(instanceId, state, mode)` | Bytes for only the pending occurrence through its checkpoint |
| `advance()` | Following parent bytes through the next occurrence or completion |
| `update(instanceId, patch)` | State-only bytes for a committed updatable occurrence |

`start`, `resume`, and `advance` return bytes, `done`, and an optional
descriptor. Interpret each step in this order:

| Step state | Required action |
|---|---|
| descriptor present | Call `resume` with that descriptor's `instanceId` |
| no descriptor and `done` is false | Call `advance` |
| `done` is true | The response is complete |

Write and flush each returned step before waiting for more data. `resume`
contains only the pending occurrence; `advance` continues through the next
occurrence or the end of the response. A completed step already includes the
remaining document bytes.

```rust
let mut response =
    handler.stream_response(&protocol, &options, &mut writer)?;
let mut step = response.start(&initial_state)?;

while !step.done {
    step = match step.boundary.as_ref() {
        Some(boundary) => {
            let state =
                load_state(&boundary.owner, &boundary.name, boundary.key.as_ref());
            response.resume(
                boundary.instance_id,
                &state,
                BoundaryMode::Final,
            )?
        }
        None => response.advance()?,
    };
}
```

`update` is also valid after a boundary-only `resume` and before its matching
`advance`. This lets the host flush a checkpoint, emit a state-only patch, and
then continue the parent.

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
locals such as component attributes and selected route context. Resume state
overlays that frozen parent state. Resolution order remains:

1. lexical locals
2. state supplied to `resume`
3. frozen parent state

## Authoring rules

- `name` is required, static, non-empty, and unique within its owner.
- Authored boundaries cannot contain another boundary, directly or through a
  component or runtime branch.
- A boundary-bearing subtree reached from a `<for>` body is rejected with
  `boundary-in-repeat`. A boundary may wrap a complete `<for>` or sit before or
  after one.
- A component-owned declaration reached from multiple static callsites in one
  entry traversal requires `key`.
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

See [Hydration](/build/hydration#progressive-streaming-hydration),
[Performance](/build/performance), and the
[integration guides](/build/integrations/) for host-specific examples.
