# Performance

WebUI moves template parsing and expression compilation to build time, then
renders the compiled protocol with request state. The biggest wins come from
shipping less state, loading less JavaScript, and asking the browser to manage
less DOM.

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

Removing unnecessary decorators reduces projected state, reactive path indexes,
and client bookkeeping.

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
[Lazy Component Policy](/guide/concepts/directives/lazy) reference. The
[Hydration](/guide/concepts/hydration) guide explains activation lifecycle and
router prefetch handoff.

## Prefer view models over repeated template decisions

Conditions are inexpensive, but hundreds of repeated branches still require
state lookups, condition evaluation, markers, and client bindings.

Instead of sending raw records and asking every row to derive the same display
state, prepare a render-focused view model on the server:

```json
{
  "title": "Ship release",
  "statusLabel": "Blocked",
  "badges": [
    { "id": "blocked", "label": "Blocked" },
    { "id": "priority", "label": "High priority" }
  ]
}
```

```html
<strong>{{todo.title}}</strong>
<span>{{todo.statusLabel}}</span>
<for each="badge in todo.badges">
  <todo-badge label="{{badge.label}}"></todo-badge>
</for>
```

This is usually cheaper than many sibling `<if>` branches and keeps business
logic out of the client runtime. Preserve real booleans and stable IDs in the
view model so interactions remain straightforward.

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

All hosts execute the native WebUI renderer, but host-side state preparation,
serialization, native-boundary calls, and HTTP dispatch still cost time.

For maximum single-core throughput:

- load and index `protocol.bin` once
- reuse immutable protocol and handler instances
- reuse request-state shapes and buffers where safe
- render directly from Rust
- avoid a JavaScript-to-native state serialization round trip

Node, Bun, Deno, Python, and other bindings remain useful when integration
cost matters more than the last increment of throughput. Measure with the
deployment host you intend to run.

## Stream slow data, not already-fast pages

Progressive boundaries help when data dependencies complete at different
times. They do not make a small, warm, buffered render intrinsically cheaper.

Use `<boundary>` when early shell bytes or independently resolving regions
improve user-visible completion. Keep ordinary pages buffered when one render
already completes quickly.

## Measure the path you are optimizing

Server and browser measurements answer different questions:

- **Single-core RPS** measures dynamic server rendering and HTML delivery.
- **FCP** and **LCP** measure browser paint milestones.
- **JS heap** measures managed JavaScript memory after readiness and garbage
  collection.
- **Renderer private memory** measures Chromium renderer-process private bytes.
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

Paint milestones are frame-scheduled and can overlap run noise. Read their
median, p95, and methodology rather than treating a small ordering difference
as a server-speed claim.

## Evidence and benchmarks

The homepage compares production paths rendering the same dynamic 100-item
application. Server throughput, browser paint, hydration, heap, and renderer
memory remain separate metrics because they answer different questions.

Use the interactive homepage explorer for the current published summary. The
complete cross-framework methodology, source, commands, and raw artifacts live
in
[microsoft/webui-benchmarks](https://github.com/microsoft/webui-benchmarks).
WebUI's internal microbenchmark and regression workflow remains documented in
[`BENCHMARKS.md`](https://github.com/microsoft/webui/blob/main/BENCHMARKS.md).
