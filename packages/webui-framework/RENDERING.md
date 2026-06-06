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

- compiled template metadata (path indices, not selectors),
- five lightweight HTML comment markers around structural blocks,
- a parallel walk of the SSR DOM and the parsed template DOM to keep ordinals aligned,
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
                        Emit __webui.state         O(affected) updates
```

1. **Server renders HTML.** The handler walks compiled template metadata and application state and emits Declarative Shadow DOM (or light DOM) with five comment markers around structural blocks, plus a `<script>` tag carrying `window.__webui.state` and the per-component template metadata.
2. **Browser parses HTML.** The parser creates shadow roots inline. The user sees a fully painted page before any framework code runs.
3. **JavaScript loads.** The component class registers via `customElements.define`. The browser upgrades pre-existing tags and fires `connectedCallback`.
4. **`$mount` decides client-or-SSR.** If a shadow root exists or the element already has children, the framework treats the DOM as SSR. Otherwise it parses the static template HTML (`meta.h`) into a detached staging root, upgrades custom elements, wires bindings, applies the first binding pass, and only then appends the nodes. Child `connectedCallback` methods see initial parent `:` property bindings.
5. **`$applySSRState` seeds observables.** Backing fields (`_count`, `_title`, ...) are written directly from `window.__webui.state` so reactive bindings observe values that match the painted DOM.
6. **`$hydrate` walks the DOM once.** Text, attribute, conditional, repeat, and event bindings are resolved by a single in-order pass that uses path indices plus marker-aware ordinal traversal.
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

Text bindings, attribute bindings, and event handlers are **not** marked. They are located via compiled path indices.

### Why markers exist for blocks but not bindings

Blocks change cardinality. A `<for>` produces zero, one, or many child runs. An `<if>` may render its content or not. The compiled path indices in `meta.h` describe the static skeleton, so the framework cannot derive "where does this block live in the SSR DOM" from path indices alone. The markers make that boundary explicit.

Static-position bindings (text, attributes, events) do not have this problem. Their position relative to the static skeleton is fixed at compile time, so a path index plus a marker-aware ordinal walk is enough.

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

Notice that there are no markers on `<h1>`, `<button>`, or the text inside `<span>`. Path indices reach those.

### Marker removal is deferred

`<!--/wc-->`, `<!--/wr-->`, and `<!--wi-->` must remain in the DOM for the **entire** hydration pass, because the ordinal-traversal algorithm uses marker pairs to skip block content when counting siblings. Removing a closing marker mid-pass corrupts later resolution calls. The framework collects them into a `staleMarkers` array and deletes them after `$finalize` (events + refs).

`<!--wc-->` and `<!--wr-->` start markers are kept after hydration as runtime anchors. They are the insertion points used when the condition flips or the repeat collection grows.

Hydration assumes SSR DOM, marker comments, and compiled metadata come from the same trusted WebUI compiler/handler version. Hand-edited marker streams are unsupported; every `<!--wr-->` and `<!--wc-->` must have its matching closing marker.

---

## Compiled template metadata

The compiler emits one `TemplateMeta` per component, delivered as a JS IIFE inside a `<script>` tag (or evaluated directly during SPA partial navigation):

```javascript
(function () {
  var w = (window.__webui || (window.__webui = {})).templates || (window.__webui.templates = {});
  w['todo-app'] = {
    h:  '<div class="todo"><ul></ul></div>',
    tx: [/* text binding runs */],
    a:  [/* attribute bindings */],
    ag: [/* attribute target groups */],
    c:  [/* conditional blocks */],
    cl: [/* conditional anchor slots */],
    r:  [/* repeat blocks */],
    rl: [/* repeat anchor slots */],
    e:  [/* event bindings and target paths */],
    b:  [/* nested block table */],
    sa: 'todo-app',
    sd: 0,
    re: [/* root host events */],
  };
})();
```

| Field | Purpose |
|---|---|
| `h` | Static HTML, marker-free, used for client-created cloning. **Never has SSR markers.** |
| `tx` | Text-binding runs, slot path + parts. |
| `a` / `ag` | Attribute bindings and the elements they target. |
| `c` / `cl` | Conditional blocks and their anchor slot paths. |
| `r` / `rl` | Repeat blocks and their anchor slot paths. |
| `e` | Event bindings with handler argument specs and target paths. |
| `b` | Nested block table (sub-templates for conditional/repeat bodies). |
| `sa` | Adopted-stylesheet specifier (CSS module). |
| `sd` | Truthy when client-created instances should attach a shadow root. |
| `re` | Root-level host events (attached to the host element, not the shadow root). |

The same metadata serves both paths:

- **SSR hydration** reads paths to compute ordinals, which are then translated against the live SSR DOM.
- **Client-created creation** clones `h` into a detached staging root, upgrades custom elements, walks paths directly, and applies initial bindings before the staged nodes are appended to the connected DOM.

### Condition AST

Conditions are compact tuples evaluated iteratively (no recursion):

| Tuple | Meaning | Example |
|---|---|---|
| `[0, path]` | Truthy check on a path | `<if condition="visible">` |
| `[1, left, op, right]` | Comparison | `<if condition="count > 0">` |
| `[2, inner]` | Logical NOT | `<if condition="!visible">` |
| `[3, left, op, right]` | AND or OR | `<if condition="a && b">` |

---

## DOM resolution: two algorithms, one metadata

### `$resolve` (client-created)

The DOM was cloned from `meta.h`, so child-node indices line up. Resolution is a flat index walk:

```typescript
let cur: Node = root;
for (const idx of path) {
  cur = cur.childNodes[idx];   // path = [1, 0] → root.childNodes[1].childNodes[0]
}
return cur;
```

### `$resolveSSR` (server-rendered)

The SSR DOM contains extra content the static template does not, specifically the rendered bodies of `<if>` and `<for>` blocks delimited by markers. Naive child-index walking would land on the wrong node after the first block.

`$resolveSSR` walks the SSR DOM and the parsed template DOM **in parallel**. At each step:

1. Look up the next template-side child's `nodeType` (element vs text) and its **ordinal among same-type siblings** in the template. This lookup is cached per-template-node in a `WeakMap` to avoid recounting.
2. Call `findByOrdinal(ssrParent, nodeType, ordinal)`, which walks SSR siblings, **skips entire `<!--wc-->...<!--/wc-->` and `<!--wr-->...<!--/wr-->` ranges** (with depth tracking for nested blocks), and returns the Nth element-or-text of the requested type.

This is why closing markers must survive the whole hydration pass: they delimit the regions to skip.

### `$findSSRText`

A specialized variant of `$resolveSSR` for text bindings. The compiler emits text-slot positions as `[parentPath, beforeIndex]` where `beforeIndex` is the static template's child index. `$findSSRText` walks SSR text-node ordinals up to that index, again skipping marker ranges.

---

## Ordinal cache

`getTplOrdinals(tplNode)` returns a `Map<childIndex, [nodeType, ordinal]>` cached in a `WeakMap` keyed by the template-DOM node. The map is built once on first access and reused for every binding inside that block.

This avoids quadratic behaviour when a block has dozens of bindings: without the cache, every binding would re-walk the parent's children to count element vs text ordinals. With the cache, each parent is walked once per block lifetime.

---

## State seeding

When the server renders `<span>42</span>` for `@observable count = 0`, the JS class default is still `0`. If the framework called `$update()` immediately, it would overwrite `42` with `0`.

`$applySSRState` runs **before** any binding is wired:

1. Read `window.__webui.state` (a JSON object emitted by the handler as a `<script>`).
2. Look up the component's `@observable` property names via the decorator registry.
3. For each key in state that matches an observable name, write directly to the backing field: `this._count = 42`. **Not** through the setter, so no reactive update fires.

After this step, `this.count === 42` matches the rendered DOM, and the subsequent hydration walk wires bindings without disturbing the painted output.

Properties not present in state, or not on the observable list, are left at their class defaults.

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

### Keyed mode

When the repeat block's root element has at least one attribute binding (e.g. `<todo-item id="{{item.id}}">`), the **first attribute** is treated as the key. On collection change:

1. Build a `Map<key, existingInstance>` from current items.
2. Walk the new collection in order. For each new item:
   - If a matching key exists, reuse the existing DOM and move it into position.
   - Otherwise, create a new instance from the block template.
3. Anything left in the map after the walk is destroyed.

Reused instances keep their event listeners, computed state, and any focus/scroll/selection state in their DOM.

### Sequential mode

When the repeat root has no attribute bindings, items are matched by index. Excess items are destroyed; new items are appended. Cheaper but loses identity on reorder.

### SSR repeat reading

On initial hydration, `$hydrate`'s repeat phase walks `<!--wi-->` markers to discover the rendered items, then runs `$hydrate` recursively on each item with a scope frame that introduces the item variable. State is already seeded from `window.__webui.state`, so item observables match the server-rendered DOM. The `<!--wi-->` markers are then collected for deletion.

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

- **Element events** (`@click="{handler(item.id, e)}"`): wired via `$wireEvents`. The compiled metadata emits `[event, handler, argSpecs, targetPath]`. Hydration resolves `targetPath` to the real element, installs the listener there, and captures the active scope frame so `argSpecs` resolve against the same repeat item or component state at dispatch time.
- **Root events** (`re` field): attached to the host element rather than the shadow root. Used for `@custom-event` on the component's `<template>` root.

Listener cleanup is automatic. `$destroy` (called from `disconnectedCallback` via a microtask, so repeat reconciliation moves don't trigger teardown) removes everything wired during `$mount`.

---

## CSS strategies

Three delivery modes, set by the compiler from `<link>` / `<style>` declarations in the source HTML:

| Strategy | How it works |
|---|---|
| **Link** | `<link rel="stylesheet">` baked into `meta.h`. The browser fetches it normally. |
| **Inline** | `<style>` element baked into `meta.h`. No extra request. |
| **Module** | A `<script type="importmap">{"imports":{"tag-name":"data:text/css,..."}}</script>` block in the page payload registers the CSS as a module. The framework retrieves the same `CSSStyleSheet` via `import(tag, { with: { type: 'css' } })` and applies it to every instance via `adoptedStyleSheets` (`meta.sa` carries the specifier). |

Module sheets are cached, so each instance pays the cost of one `adoptedStyleSheets` push, not a full CSS parse.

---

## Light DOM vs Shadow DOM

Set by the compiler via `--dom` flag, surfaced as `meta.sd`:

- **Shadow DOM** (`meta.sd` truthy): SSR uses Declarative Shadow DOM. Client-created instances call `attachShadow({ mode: 'open' })`. Slot content stays in light DOM and projects through.
- **Light DOM**: SSR renders children directly into the host. Client-created instances populate the host's `appendChild` slot. No style isolation; CSS lives globally or on the host.

`$mount` auto-detects:

- `this.shadowRoot` present → shadow DOM SSR.
- Children present and `meta.sd` not set → light DOM SSR.
- `meta.sd` set, no shadow root → shadow DOM client-created (existing children become slot content).
- Otherwise → light DOM client-created.

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

The `webui:hydration-complete` event fires once after the last component on the page finishes. Use it to gate post-hydration logic or to ship a metric.

---

## Performance characteristics

| Operation | Cost | Why |
|---|---|---|
| Initial hydration | O(bindings) | Single pass over compiled paths |
| Reactive update | O(affected) | Path index skips unrelated bindings |
| Conditional toggle | O(block size) | Create or destroy a block instance |
| Repeat reconciliation (keyed) | O(items) | Map lookup per item, in-place moves |
| Repeat reconciliation (sequential) | O(items) | Linear scan, append/remove tail |
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
│                               $resolve, $resolveSSR, $update, events,
│                               teardown, path index
├── element/
│   ├── markers.ts              Marker constants, collectItemMarkers,
│   │                           findByOrdinal (block-skipping ordinal walk)
│   ├── diff.ts                 syncRepeat: keyed + sequential reconciliation
│   ├── styles.ts               injectModuleStyle (adopted CSS modules)
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
- "Template metadata not found": the `<script>` tag carrying the IIFE is missing from the page. Check the build output.
- A binding that does not update: confirm the property is `@observable` (not just a class field) and the path appears in the template. Check `$pathIndex` after the first update if you can attach a debugger.

---

## Where to look next

- `examples/app/todo-webui` — minimal SSR + interactivity example
- `examples/app/contact-book-manager` — repeat blocks, keyed reconciliation
- `examples/app/commerce` — larger composition, multiple components per page
- [Interactivity guide](https://microsoft.github.io/webui/guide/concepts/interactivity) — component-author view of the same machinery
