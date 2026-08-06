<!--
Copyright (c) Microsoft Corporation.
Licensed under the MIT license.
-->

# Rendering & Hydration Internals

How `@microsoft/webui-framework` actually turns server-rendered HTML into a live, reactive DOM, and what it does on every keystroke after that.

This document is for framework contributors, plugin authors, and anyone debugging hydration. **If you just want to author components, read [`README.md`](./README.md) and the [Interactivity guide](https://microsoft.github.io/webui/guide/concepts/interactivity) instead.**

---

## Why a separate document

WebUI is built on a hard rule: the server emits HTML, the browser parses HTML, and the framework adopts that HTML in place. Nothing is re-rendered. No virtual DOM, no diff against a fresh tree, no `innerHTML = ...` to swap content. To make that work without DOM annotations on every dynamic node, the framework leans on:

- compiled template metadata (element indices, not selectors),
- five lightweight HTML comment markers around structural blocks,
- a single pre-order walk pairing the SSR DOM with the parsed template DOM,
- a per-component path index so reactive updates touch only the bindings that actually depend on a changed property.

The rest of this document explains each of those pieces, in the order the runtime executes them.

---

## Lifecycle at a glance

```
Build time              Server render              Client hydration
──────────────          ────────────────           ──────────────────
Parse templates   →     Render with state    →     Framework adopts
Compile metadata        Inject SSR markers         existing DOM,
                        Emit Declarative           wires bindings,
                        Shadow DOM                 strips markers
                        Emit webui-data            O(affected) updates
```

1. **Server renders HTML.** The handler walks compiled template metadata and application state and emits Declarative Shadow DOM (or light DOM) with five comment markers around structural blocks, plus an inert `#webui-data` block carrying state and per-component template metadata.
2. **Browser parses HTML.** The parser creates shadow roots inline. The user sees a fully painted page before any framework code runs.
3. **JavaScript loads.** The component class registers via `customElements.define`. The browser upgrades pre-existing tags and fires `connectedCallback`.
4. **`$mount` decides client-or-SSR.** If a shadow root exists or the element already has children, the framework treats the DOM as SSR. Otherwise it parses the static template HTML (`meta.h`) into a detached staging root, upgrades custom elements, wires bindings, applies the first binding pass, and only then appends the nodes. Child `connectedCallback` methods see initial parent `:` property bindings.
5. **`$applySSRState` seeds observables.** Backing fields (`_count`, `_title`, ...) are written directly from `window.__webui.state` so reactive bindings observe values that match the painted DOM.
6. **`$hydrate` walks the DOM once.** One marker-aware pre-order pass numbers the subtree; text, attribute, conditional, repeat, and event bindings then resolve by index. The attribute pass also transfers known complex `:` values. An unupgraded compiled WebUI child holds those values behind a weak key and consumes them after bootstrap but before its own binding walk, without a promise or strong retained element reference.
7. **Stale markers are removed.** Item markers (`<!--wi-->`) and closing markers (`<!--/wc-->`, `<!--/wr-->`) are deleted; start markers (`<!--wc-->`, `<!--wr-->`) stay as anchors for runtime updates.
8. **Path index is built lazily on the first reactive change.** Subsequent updates are O(affected bindings).

There is no flash of content, because the HTML was already on screen at step 2. There is no first render, because the framework never re-renders the DOM that SSR emitted.

---

## SSR markers

The handler emits exactly five comment markers, all defined in `src/element/markers.ts`:

| Marker | Meaning |
|---|---|
| `<!--wr-->` | Repeat block start (one per `<for>`) |
| `<!--/wr-->` | Repeat block end |
| `<!--wi-->` | Repeat item boundary (one per iteration) |
| `<!--wc-->` | Conditional block start (one per `<if>`) |
| `<!--/wc-->` | Conditional block end |

Text bindings, attribute bindings, and event handlers are **not** marked. They are located via compiled element indices.

### Why markers exist for blocks but not bindings

Blocks change cardinality. A `<for>` produces zero, one, or many child runs. An `<if>` may render its content or not. The compiled indices in `meta.h` describe the static skeleton, so the framework cannot derive "where does this block live in the SSR DOM" from those indices alone. The markers make that boundary explicit.

Static-position bindings (text, attributes, events) do not have this problem. Their position relative to the static skeleton is fixed at compile time, so a pre-order element index is enough.

### Example

Template:

```html
<h1>{{title}}</h1>
<button @click="{toggle()}">Toggle</button>
<if condition="visible">
  <p>Now you see me</p>
</if>
<for each="item in items">
  <span data-id="{{item.id}}">{{item.name}}</span>
</for>
```

Server output:

```html
<template shadowrootmode="open">
  <h1>My List</h1>
  <button>Toggle</button>
  <!--wc--><p>Now you see me</p><!--/wc-->
  <!--wr-->
    <!--wi--><span data-id="1">Alice</span>
    <!--wi--><span data-id="2">Bob</span>
  <!--/wr-->
</template>
```

Notice that there are no markers on `<h1>`, `<button>`, or the text inside `<span>`. Pre-order element indices reach those.

### Marker removal is deferred

`<!--/wc-->`, `<!--/wr-->`, and `<!--wi-->` must remain in the DOM for the **entire** hydration pass, because the walk uses marker pairs to skip block content. Removing a closing marker mid-pass corrupts later resolution calls. The framework collects them into a `staleMarkers` array and deletes them after `$finalize` (events + refs).

`<!--wc-->` and `<!--wr-->` start markers are kept after hydration as runtime anchors. They are the insertion points used when the condition flips or the repeat collection grows.

The removed closing markers make structural SSR hydration intentionally
one-shot. If delayed disconnect cleanup destroys a hydrated binding graph,
reconnect remounts templates containing conditions or repeats from current
component state rather than trying to claim the marker-stripped DOM again.

Hydration assumes SSR DOM, marker comments, and compiled metadata come from the same trusted WebUI compiler/handler version. Hand-edited marker streams are unsupported; every `<!--wr-->` and `<!--wc-->` must have its matching closing marker.

---

## Compiled template metadata

The compiler emits one JSON-safe `TemplateMeta` per component plus a small component-local condition closure array. During SSR, all non-executable metadata is delivered in `<script type="application/json" id="webui-data">`; during SPA partial navigation, the router registers the metadata object directly and executes only the closure arrays.

```json
{
  "inventory": "01",
  "state": { "items": [] },
  "templates": {
    "todo-app": {
      "h": "<div class=\"todo\"><ul></ul></div>",
      "tx": [],
      "a": [],
      "ag": [],
      "c": [[[0, ["items.length"]], 0, [[], 0]]],
      "r": [["items", "item", 1, [[0], 0]]],
      "eg": [],
      "b": [],
      "sd": 1,
      "re": []
    }
  }
}
```

The matching executable payload is stored under `window.__webui.templateFns['todo-app']`, for example `[function(v,s){return !!v("items.length",s)}]`. The framework normalizes `[functionIndex, paths]` condition references into direct `[fn, paths]` tuples once before hydration.

| Field | Purpose |
|---|---|
| `h` | Static HTML, marker-free, used for client-created cloning. **Never has SSR markers.** |
| `tx` | Text-binding runs, slot path + parts. |
| `a` / `ag` | Attribute bindings and the elements they target. |
| `c` | Conditional blocks with `[conditionRef, blockIndex, slot]`. |
| `r` | Repeat blocks with `[collection, itemVar, blockIndex, slot]`. |
| `eg` | Event bindings grouped by event name, with handler argument specs and target paths. |
| `b` | Nested block table (sub-templates for conditional/repeat bodies). |
| `sd` | Truthy when client-created instances should attach a shadow root. |
| `re` | Root-level host events (attached to the host element; observe events targeted at the host itself plus anything bubbling to it - `composed` is required only to cross a shadow boundary). |

The same metadata serves both paths:

- **SSR hydration** numbers the live SSR DOM in the same pre-order the compiler numbered the template, skipping structural block ranges.
- **Client-created creation** clones `h` into a detached staging root, upgrades custom elements, numbers it with a plain pre-order walk, and applies initial bindings before the staged nodes are appended to the connected DOM.

### Condition references

Conditions are stored in JSON as `[functionIndex, paths]`. `functionIndex`
points into `window.__webui.templateFns[tagName]`, and `paths` drives the
reactive path index. The framework normalizes this once to `[fn, paths]`
before hydration or client-created wiring.

```javascript
// Metadata
[0, ['visible']]

// Function table
[function(v, s) { return !!v('visible', s); }]
```

---

## DOM resolution: one numbering, two walks

Every locator names an element by its **pre-order index** within its own
compiled section: `0` is the section root, and elements are numbered `1..N` in
the order a depth-first walk of `h` meets them. The root template and each
`<if>` / `<for>` block number independently, matching the `b[]` split.

Both paths rebuild that numbering in a single walk, then resolve every binding
by array index.

### `collectTemplateElements` (client-created)

The DOM was cloned from `meta.h`, so it matches the template node for node. A
plain pre-order walk reproduces the compiled indices - no markers are involved
and nothing is skipped.

### `buildSSRIndex` (server-rendered)

The SSR DOM contains extra content the static template does not: the rendered
bodies of `<if>` and `<for>` blocks, delimited by markers. The walk pairs the
template with the server output in lockstep and **skips entire
`<!--wc-->...<!--/wc-->` and `<!--wr-->...<!--/wr-->` ranges** (with depth
tracking for nested blocks), because that content belongs to the block's own
metadata. This is why closing markers must survive the whole hydration pass:
they delimit the regions to skip.

The same pass collects `<!--wc-->` and `<!--wr-->` markers in document order.
The compiler emits `c` / `r` in source order and the server renders in source
order, so the two line up by index - which is what makes each block's anchor
unambiguous.

Two details shape the walk:

- The counter follows the **template**, never the server output. A run of
  missing SSR elements leaves holes rather than shifting every later index onto
  the wrong node.
- It descends where the template has children, and also into a template-empty
  element when the section has blocks to place - `<ul><for …></ul>` compiles to
  an empty `<ul>`. Child components are the exception: they contribute no
  children to the parent's `h`, so whatever the server rendered inside belongs
  to them.

### `$findSSRText`

Text is the one thing that cannot be indexed. The renderer strips
inter-element whitespace that `meta.h` keeps, so text nodes do not line up
positionally even though elements do. The compiler emits text-slot positions as
`[parentIndex, beforeIndex]`, and `$findSSRText` walks SSR text-node ordinals up
to that index, skipping marker ranges.

---

## State seeding

When the server renders `<span>42</span>` for `@observable count = 0`, the JS class default is still `0`. If the framework called `$update()` immediately, it would overwrite `42` with `0`.

`$applySSRState` runs **before** any binding is wired:

1. Read `window.__webui.state` (loaded lazily from the handler-emitted `#webui-data` block).
2. Look up the component's `@observable` property names via the decorator registry.
3. For each key in state that matches an observable name, write directly to the backing field: `this._count = 42`. **Not** through the setter, so no reactive update fires.

After this step, `this.count === 42` matches the rendered DOM, and the subsequent hydration walk wires bindings without disturbing the painted output.

Properties not present in state, or not on the observable list, are left at their class defaults.

Compiler-owned dormant hosts follow a stricter first-write rule: activation
wires the SSR DOM, then updates only roots explicitly supplied by that write.
Omitted text, attribute, conditional, and repeat roots retain their trusted SSR
output until state supplies those roots.

---

## Reactive update model

After hydration, every dynamic value is connected to a direct DOM-node reference inside a binding object. There is no virtual DOM, no `querySelector`, and no diffing.

### Path index

`$buildPathIndex` (called lazily on the first `$update`) walks every binding in the component and groups them by the observable property names they depend on:

```text
'count'  → { texts: [t1, t2], attrs: [], conds: [c1], repeats: [] }
'title'  → { texts: [t3],     attrs: [a1], conds: [], repeats: [] }
'*'      → { texts: [...],    attrs: [...], conds: [...], repeats: [...] }   // volatile/computed
```

The wildcard (`'*'`) bucket holds bindings whose expressions reference a path the framework cannot pre-classify (typically computed getters). They run on every flush.

### Update flow

```
this.count = 5
  → @observable setter writes _count, calls $update('count')
  → $update queues 'count' on $dirtyPaths, schedules a microtask
  → $flush walks $dirtyPaths once, looking each path up in $pathIndex
  → for each entry, walks only that subset of bindings
  → wildcard bindings run once per flush (not per dirty path)
  → DOM is patched via direct .textContent / setAttribute / etc.
```

Updates are coalesced via `queueMicrotask`. Multiple synchronous setter calls inside a single tick produce one DOM pass.

### `$flushUpdates()`

Synchronous escape hatch. Call it when you need the DOM to reflect pending writes immediately (test code, measurement before paint, etc.).

### Why this is fast

- `$pathIndex.get(name)` is an O(1) `Map` lookup.
- Each binding holds a direct `Text`/`Element` reference resolved during hydration. No selectors run on update.
- Skipping unrelated bindings means a 200-binding component pays the cost of the 3 bindings that actually depend on `count`.
- No tree walk, no diff, no allocation per update beyond the `Set<string>` of dirty paths.

---

## Repeat reconciliation (`<for>`)

Implemented in `src/element/diff.ts`.

### Positional mode (default)

Every repeat matches items by array index:

1. Rebind the shared prefix of existing instances to the current items.
2. Append instances for any new tail.
3. Destroy instances in any excess old tail.

Repeated-root attributes are never inferred as keys. Duplicate values and
attributes are therefore safe, and attribute order has no effect on identity.
On reorder, reused instances keep local browser and component state at their
positions while bindings update to the new positional items.

### Explicit-key mode

`<for each="item in items"><x key="{{item.id}}"></x></for>` compiles the
relative path `id` from the first child as an optional fifth repeat metadata
field. `key="{{item}}"` compiles an empty path and keys primitive items
directly. `key` is compiler-only: it is omitted from SSR HTML, client `h`, and
attribute metadata. `data-key` is an ordinary application attribute and has no
identity semantics. Unkeyed repeat bindings do not allocate key state.

Explicit keys must resolve to unique strings or finite numbers. The runtime
validates the complete next key set before changing DOM, scopes, or instances.
Stable order, append, and truncate use the positional/prefix fast path. A real
order change fills one reusable map from old keys to instances, reorders the
instances, and then clears the map and scratch arrays.

Duplicate, invalid, or throwing key reads clear established identity, warn
once, and use positional reconciliation. A later valid update first reconciles
positionally and establishes fresh identity; subsequent updates can move by
key.

### SSR repeat reading

On initial hydration, `$hydrate`'s repeat phase walks `<!--wi-->` markers to
discover the rendered items, then runs `$hydrate` recursively on each item with
a scope frame that introduces the item variable. When the repeat collection is
present in client state, that frame is synchronized immediately. When the
collection is template-only and intentionally absent from bootstrap state, the
frame remains unknown and its SSR bindings are preserved during unrelated
updates. A later explicit collection reconciles the repeat normally; an
explicit empty collection removes the SSR items. The `<!--wi-->` markers are
then collected for deletion.

SSR item markers do not contain separate key values. When bootstrap collection
state exists and its length matches the hydrated instance count, hydration
derives typed keys by index from that collection. Missing state, a count
mismatch, or invalid keys leave identity unestablished, so the next valid
update reconciles positionally once before establishing fresh keys. This uses
the same invariant as repeat scope hydration: SSR HTML and bootstrap state
represent the same render.

---

## Conditional reconciliation (`<if>`)

The `<!--wc-->` start marker is the runtime anchor. On hydration:

1. Evaluate the condition tuple against the resolver. If truthy and an SSR marker pair exists, recursively `$hydrate` the content between the markers.
2. If falsy, the SSR pair already contains nothing the framework cares about. The closing marker is queued for removal; the opening marker is kept as the anchor.

On reactive flip:

- `false → true`: clone the block template under the anchor, wire it via the client-created path, run an immediate flush.
- `true → false`: tear down the existing `TemplateInstance`, remove its nodes, keep the anchor.

---

## Events

Two flavours:

- **Element events** (`@click="{handler(item.id, e)}"`): wired via `$wireEvents`. The compiled metadata emits `eg` groups shaped as `[event, [[handler, argSpecs, targetPath, usesEvent?]]]`. Hydration resolves `targetPath` to the real element and captures the active scope frame so `argSpecs` resolve against the same repeat item or component state at dispatch time. Listeners attach to the bound element, so `event.currentTarget` is correct and `stopPropagation()` behaves as authored.

  Bindings are never delegated to the render root. `$wireEvents` runs once per block instance and the render root is shared across instances, so delegating would stack one listener per block on the same node and fire all of them per dispatch — O(N) for no reduction in listener count. It would also miss non-bubbling events (`focus`, `blur`, `mouseenter`, `load`, `error`, `toggle`, media) and app-defined events dispatched without `bubbles: true`, which no shipped event-name table can cover.
- **Root events** (`re` field): used for `@custom-event` on the component's `<template>` root. Attached to the **host element**, the only node that observes both events dispatched on the host itself (which never enter the shadow tree) and `composed` events on their way out of it. Non-composed events (`change`, `submit`, `select`, media) stop at the shadow root by design and are bound per element instead. Because the listener is on the host, `event.target` is retargeted to the host for inner events — use `event.composedPath()[0]` to recover the originating element.

Listener cleanup is automatic. `$destroy` (called from `disconnectedCallback` via a microtask, so repeat reconciliation moves don't trigger teardown) removes everything wired during `$mount`.

---

## CSS strategies

Three delivery modes are set by the compiler:

| Strategy | How it works |
|---|---|
| **Link** | Installs an external stylesheet resource |
| **Style** | Installs compiled CSS in a `<style>` element |
| **Module** | Starts with an SSR style fallback, then imports and adopts a shared `CSSStyleSheet` |

Compiler-ordered closures install resources once per Document or ShadowRoot.
Partial navigation, progressive streaming, and component assets share the same
required `componentStyles` catalog. Module resources carry their CSS directly;
the catalog installs each specifier's import map once per owning Document
before importing it, then reuses the cached parsed sheet for every target.

---

## Light DOM vs Shadow DOM

Every unwrapped component uses Light DOM. A sole top-level
`<template shadowrootmode="open">` containing the complete component selects
Shadow. This is surfaced as `meta.sd`:

- **Shadow DOM** (`meta.sd` truthy): SSR uses Declarative Shadow DOM. Client-created instances call `attachShadow({ mode: 'open' })`. Slot content stays in light DOM and projects through.
- **Light DOM**: SSR renders children directly into the host. Client-created
  instances populate the host. Compiler-scoped CSS is installed in the owning
  Document, and native `<slot>` is rejected at build time.

`$mount` auto-detects:

- `this.shadowRoot` present → shadow DOM SSR.
- Children present and `meta.sd` not set → light DOM SSR.
- `meta.sd` set, no shadow root → shadow DOM client-created (existing children become slot content).
- Otherwise → light DOM client-created.

Style resources follow compiler-ordered closures and install once per Document
or ShadowRoot. A ShadowRoot is a closure cut point.

---

## Performance instrumentation

`src/lifecycle.ts` integrates with the [Performance API](https://developer.mozilla.org/en-US/docs/Web/API/Performance_API):

| Mark | When |
|---|---|
| `webui:hydrate:total:start` | First component begins hydrating |
| `webui:hydrate:total:end` | Last component finishes |
| Measure `webui:hydrate:total` | Total wall-clock hydration time |

```typescript
window.addEventListener('webui:hydration-complete', () => {
  const entry = performance.getEntriesByName('webui:hydrate:total', 'measure')[0];
  if (entry) console.log(`Hydration: ${entry.duration.toFixed(1)}ms`);
});
```

The `webui:hydration-complete` event fires once after the parser-startup
hydration cohort settles. For lazy roots, the cohort waits for the first
intersection result: initially visible roots finish first, while dormant roots
do not keep the event open or redispatch it later. Use `hydratedCallback()` for
per-instance readiness.

---

## Performance characteristics

| Operation | Cost | Why |
|---|---|---|
| Initial hydration | O(bindings) | Single pass over compiled paths |
| Reactive update | O(affected) | Path index skips unrelated bindings |
| Conditional toggle | O(block size) | Create or destroy a block instance |
| Repeat reconciliation | O(items) | Positional scan; keyed map only for changed explicit-key order |
| Event wiring | O(events) | One-time during hydration |

### What the framework does NOT do

- No virtual DOM, no tree copy, no diff algorithm.
- No `innerHTML` on updates. Only `textContent` and `setAttribute`.
- No `querySelector` on updates. All node references are pre-resolved.
- No recursion in hot paths. Conditions evaluate on an explicit stack.
- No runtime template parsing. The compiler does all syntax work ahead of time.

---

## Module map

```
src/
├── element.ts                  Orchestrator: $mount, $hydrate, $wire,
│                               $wire, $hydrate, $update, events,
│                               teardown, path index
├── element/
│   ├── markers.ts              Marker constants, collectItemMarkers,
│   │                           buildSSRIndex (block-skipping pre-order walk)
│   ├── diff.ts                 syncRepeat: positional + explicit-key reconciliation
│   ├── styles.ts               componentStyles catalog, installComponentStyles
│   │                           (Link/Style/Module resources, import maps)
│   └── types.ts                AttrBinding, CondBinding, RepeatBinding,
│                               TextBinding, ScopeFrame, TemplateInstance
├── decorators.ts               @observable, @attr, attribute name registry,
│                               toKebabCase fast path
├── template.ts                 TemplateMeta types + getTemplate registry
├── lifecycle.ts                Hydration timing, hydration-complete event
└── index.ts                    Public surface
```

Public exports:

```typescript
export { WebUIElement } from './element.js';
export { observable, attr } from './decorators.js';
export { getTemplate, type TemplateMeta } from './template.js';
export { hydrationStart, hydrationEnd } from './lifecycle.js';
```

Everything else is internal and may change without notice.

---

## Debugging

- Performance: `performance.getEntriesByName('webui:hydrate:total', 'measure')` after `webui:hydration-complete`.
- Per-component lifecycle: instrument `connectedCallback` / `disconnectedCallback` on a subclass.
- Marker layout: View Source on the SSR HTML. The five comment markers should be balanced; mismatched pairs almost always indicate a handler-plugin bug.
- "Template metadata not found": `window.__webui.templates` was not populated from `#webui-data` or partial-response template registration. Check the build output.
- A binding that does not update: confirm the property is `@observable` (not just a class field) and the path appears in the template. Check `$pathIndex` after the first update if you can attach a debugger.

---

## Where to look next

- `examples/app/todo-webui` — minimal SSR + interactivity example
- `examples/app/contact-book-manager` — repeat block reconciliation
- `examples/app/commerce` — larger composition, multiple components per page
- [Interactivity guide](https://microsoft.github.io/webui/guide/concepts/interactivity) — component-author view of the same machinery
