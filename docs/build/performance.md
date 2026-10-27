---
description: Budget WebUI server rendering, browser code, state payloads, and DOM work, then measure the complete application.
---

# Performance

WebUI moves template parsing and expression compilation to build time, then
renders the compiled protocol with request state. This page is a practical
optimization guide, not a cross-framework ranking. For the historical dataset
and its limitations, see [Benchmarks](/reference/benchmarks/).

Start by measuring your application. Then reduce the state, JavaScript, or DOM
work responsible for its bottleneck.

## Keep components scriptless by default

A component needs a `.ts` file only when it owns browser behavior:

- an event handler
- a `w-ref` used by an imperative browser API
- lifecycle work
- a network request
- a public property or method used by another module

Bindings, `<if>`, and `<for>` work without authored JavaScript. Scriptless
components use compiler-owned hosts and do not need empty `WebUIElement`
classes.

Use `@observable` only when TypeScript reads or writes the value after
hydration. Use `@attr` only for a public component attribute. Template-only
values belong in server state.

```typescript
// Needed: TypeScript changes count after a click.
@observable count = 0;

increment(): void {
  this.count += 1;
}
```

```json
{
  "heading": "Rendered by the server",
  "description": "No client observable needed"
}
```

Removing unnecessary decorators can reduce projected state and client
bookkeeping. Scriptless HTML still has transfer, parsing, layout, paint, and
memory costs.

## Project only the state hydration needs

Pass the application bundler's projection manifest to `webui build`:

```bash
webui build ./src \
  --out ./dist \
  --plugin webui \
  --projection-manifest ./dist/webui-projection.json
```

The manifest narrows initial state to the `@observable` and `@attr` roots used
by authored client code. Without it, WebUI preserves full state for
compatibility.

Projection is a payload optimization, not a secrecy boundary. Never place
credentials or private tokens in render state.

Keep state route-scoped as well. A page that renders one account should not
receive the application's complete account collection.

For [desktop apps](/build/desktop), keep canonical collections in
shared Rust storage and borrow them for read-only route preparation. Keep
global seed settings separate from those collections. Loading browser-derived
copies of the same data only to recompute them in Rust adds startup allocations
without improving the page.

## Defer only work that can safely wait

Keep visible, first-use-critical UI eager. Move work off the startup path only
when the product can accept a later activation:

- Use lazy rendering for repeated or numerous offscreen rows/cards. It reduces
  browser layout/paint work and delays hydration until relevance.
- Use hydration-only deferral when rendering containment could break layout.
- Use interaction hydration for one optional shell or island when startup
  JavaScript/heap matter more than cold first-use latency.
- Combine lazy rendering and interaction hydration only for one optional
  offscreen singleton. Do not use interaction boundaries for repeated items.

The exact syntax, required size reservation, combinations, and instance
overrides live in the
[Lazy Component Policy](/learn/directives/lazy) reference. The
[Hydration](/build/hydration) guide explains activation lifecycle and
router prefetch handoff.

## Prefer view models over repeated template decisions

Conditions are inexpensive, but hundreds of repeated branches still require
state lookups, condition evaluation, markers, and client bindings.

Instead of sending raw records and asking every row to derive the same display
state, prepare a render-focused view model on the server:

```json
{
  "todo": {
    "title": "Ship release",
    "statusLabel": "Blocked",
    "badges": [
      { "id": "blocked", "label": "Blocked" },
      { "id": "priority", "label": "High priority" }
    ]
  }
}
```

```html
<strong>{{todo.title}}</strong>
<span>{{todo.statusLabel}}</span>
<for each="badge in todo.badges">
  <todo-badge key="{{badge.id}}" label="{{badge.label}}"></todo-badge>
</for>
```

This keeps business logic out of the client runtime. Compare the resulting
payload and rendered work rather than assuming a larger view model is always
cheaper. Preserve real booleans and stable IDs so interactions remain
straightforward.

<span id="light-dom-vs-shadow-dom"></span>

## Choose Light and Shadow DOM deliberately

Use Light DOM for shells, document layout, and large repeated surfaces where
global CSS is intentional. It avoids per-root wrappers and lets one stylesheet
serve the page.

Use Shadow DOM selectively when a component needs strong style isolation,
slots, or host encapsulation. Every declarative Shadow root adds markup and
style ownership work, so deeply repeated leaf components should earn that
boundary.

Use the same CSS strategy when comparing the two modes. Link CSS keeps
stylesheets cacheable and avoids repeating complete CSS text in every Shadow
root:

```bash
webui build ./src --out ./dist --plugin webui --css link --dom light
webui build ./src --out ./dist-shadow --plugin webui --css link --dom shadow
```

A common architecture is a Light DOM application shell with selected Shadow DOM
widgets where encapsulation provides concrete value.

## Prefer Rust on the hottest SSR path

Supported hosts use the WebUI renderer, but host-side state preparation,
serialization, native-boundary calls, and HTTP dispatch still cost time.

When rendering is the measured bottleneck:

- load and index `protocol.bin` once
- reuse the loaded immutable protocol and host resources where supported
- reuse request-state shapes and buffers where safe
- consider direct Rust rendering to avoid a host-to-native serialization step

Node, Bun, Python, and other bindings remain useful when integration
cost matters more than removing a host boundary. Measure cold and warm
requests, including data access and serialization, with the deployment host
you intend to run. A native renderer does not eliminate your host's own costs.

## Stream slow data, not already-fast pages

Progressive boundaries help when data dependencies complete at different
times. They do not make a small, warm, buffered render intrinsically cheaper.

Use `<boundary>` when early shell bytes or independently resolving regions
improve user-visible completion. Keep ordinary pages buffered when one render
already completes quickly.

For Rust streaming integrations, move freshly loaded state into
`StreamingResponse::start` and `resume` when the caller no longer needs it.
Passing owned state avoids cloning its retained projection. Keep borrowed
state when caller ownership is required. See [Rust streaming SSR](/build/integrations/rust#streaming-ssr).

## Measure the path you are optimizing

Server and browser measurements answer different questions:

- **RPS and latency** describe the tested server, concurrency, response, and
  load profile. Record the CPU allocation and response mode.
- **CPU per request and server RSS** describe server work and process memory,
  not browser cost.
- **FCP** and **LCP** measure browser paint milestones.
- **JS heap** measures managed JavaScript memory. Report the sampling point
  and whether the harness forced garbage collection.
- **Renderer private memory** measures Chromium renderer-process private bytes;
  it is not interchangeable with JS heap.
- **Request to hydrated** includes the dynamic server response, HTML transfer,
  external assets, JavaScript execution, and startup hydration.

Use the built-in hydration marks for local investigation:

```javascript
window.addEventListener("webui:hydration-complete", () => {
  for (const entry of performance.getEntriesByType("measure")) {
    if (entry.name.startsWith("webui:hydrate:")) {
      console.log(entry.name, entry.duration);
    }
  }
});
```

Use the same browser, viewport, CPU/network settings, cache state, and assets
for before/after comparisons. Record multiple runs and variability. A single
server round cannot estimate run-to-run uncertainty, and browser navigations
are not extra server-throughput rounds.

Paint milestones are frame-scheduled and can overlap run noise. Do not treat a
small ordering difference as a server-speed claim. Lab observations also do
not establish real-user Core Web Vitals compliance.

## Evidence and benchmarks

The dedicated [benchmark page](/reference/benchmarks/) preserves the available dataset as
historical, limited evidence. It discloses the single measured server round,
ten browser navigations, selectively refreshed WebUI rows, and missing public
provenance. Do not use it as proof of a current framework winner.

A publishable comparison needs inspectable source and revisions, versions,
environment, workload, response mode, sample counts, uncertainty, exclusions,
and raw artifacts. Keep server and browser metrics separate.
