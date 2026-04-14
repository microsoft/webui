# Hydration

## What is Hydration?

WebUI compiles templates at build time and renders HTML at runtime (server-render time) with **zero JavaScript**. The browser displays content immediately via [Declarative Shadow DOM](https://developer.chrome.com/docs/css-ui/declarative-shadow-dom) - users see a fully rendered page before any script loads.

**Hydration** is the process of attaching event listeners and reactive bindings to that already-rendered DOM. This is _not_ re-rendering: the DOM already exists. Hydration makes it interactive.

WebUI uses an **islands architecture**: only interactive components ship JavaScript. If a page has ten components but only two need click handlers, only those two ship a framework. Everything else stays as static HTML with zero runtime cost.

::: tip Authoring vs. mechanism
This page explains **how hydration works** under the hood - markers, DOM walks, and metadata. For how to _write_ interactive components (decorators, events, refs, template syntax), see [Interactivity](/guide/concepts/interactivity).
:::

## How Hydration Works

```
Build time              Server render              Client hydration
──────────────          ────────────────           ──────────────────
Parse templates   →     Render with state    →     Framework reconnects
Compile metadata        Inject SSR markers         bindings & events
                        Emit Declarative           Remove markers
                        Shadow DOM
```

The lifecycle in detail:

1. **Server renders HTML** - the handler evaluates templates with application state, emitting Declarative Shadow DOM with SSR markers (binding indices, event counts).
2. **Browser parses HTML** - the parser creates shadow roots inline. Content is visible immediately, with no layout shift.
3. **JavaScript loads** - `<script>` tags deliver the component class and its compiled template metadata.
4. **Custom elements upgrade** - the browser calls `connectedCallback`. The framework detects the _existing_ shadow root instead of creating a new one.
5. **Single DOM walk** - the framework walks the shadow DOM once, using path-indexed markers to reconnect text bindings, attribute bindings, conditionals, repeats, and event listeners.
6. **State seeded automatically** - observable properties are restored from the SSR markers during the walk. No manual DOM reading required.
7. **Markers removed** - SSR-only markers are stripped from the DOM. The component is fully interactive.

No flash of content - HTML is already visible from step 2, and hydration silently wires up reactive state behind it.

## Template Metadata

At build time, the WebUI compiler produces a metadata object for each component as a raw JS IIFE string. During SSR, the handler wraps all templates into a single `<script>` tag. During SPA partial navigation, the router evaluates them directly — no `<script>` wrapper needed.

```javascript
(function () {
  var w = window.__webui_templates || (window.__webui_templates = {});
  w['todo-app'] = {
    h:  '<div class="todo"><ul></ul></div>',  // Marker-free HTML for client-created DOM
    tx: [/* text binding runs */],
    a:  [/* attribute bindings */],
    ag: [/* attribute group targets */],
    c:  [/* conditional blocks */],
    cl: [/* conditional anchor slots */],
    r:  [/* repeat blocks */],
    rl: [/* repeat anchor slots */],
    e:  [/* event bindings */],
    el: [/* event target paths */],
    b:  [/* nested block table (conditional/repeat bodies) */],
    sa: 'todo-app',   // Adopted stylesheet specifier
    re: [/* root-level host events */],
  };
})();
```

| Field | Purpose |
|-------|---------|
| `h` | Static HTML string - **marker-free** - used when creating components on the client (not during hydration) |
| `tx` | Text binding runs: maps each `{{expression}}` to a DOM path and property references |
| `a` / `ag` | Attribute bindings and their element targets |
| `c` / `cl` | Conditional blocks (`<if>`) and their anchor positions |
| `r` / `rl` | Repeat blocks (`<for>`) and their anchor positions |
| `e` / `el` | Event bindings (`@click`, etc.) and the DOM paths to their target elements |
| `b` | Nested block table - sub-templates for conditional and repeat bodies |
| `re` | Root events - attached to the host element, not the shadow root |

The same metadata serves two purposes:

- **SSR hydration** - reconnect bindings to the existing DOM using markers
- **Client-side creation** - clone `h` and resolve binding paths directly (no markers needed)

## SSR Markers

The handler injects lightweight markers into rendered HTML so the framework knows where to attach bindings. Markers appear **only inside component shadow roots** - the root page scope stays marker-free.

### Marker reference

| Marker | Format | Purpose |
|--------|--------|---------|
| Repeat block start | `<!--wr-->` | Opens a `<for>` loop region |
| Repeat block end | `<!--/wr-->` | Closes the `<for>` loop region |
| Repeat item | `<!--wi-->` | Marks each iteration boundary |
| Conditional start | `<!--wc-->` | Opens an `<if>` block |
| Conditional end | `<!--/wc-->` | Closes the `<if>` block |

The WebUI Framework plugin emits only these five comment markers. Text bindings, attribute bindings, and event handlers are resolved from compiled metadata path indices - no DOM markers needed for those.

### Example: rendered HTML with markers

Given this template:

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

The server renders something like:

```html
<template shadowrootmode="open">
  <h1>My List</h1>
  <button>Toggle</button>
  <!--wc--><p>Now you see me</p><!--/wc-->
  <!--wr-->
    <!--wi--><span>Alice</span>
    <!--wi--><span>Bob</span>
  <!--/wr-->
</template>
```

### How the framework uses markers

During the single DOM walk:

1. **Text bindings** - the framework resolves template node paths to SSR text nodes and attaches reactive subscriptions. No SSR markers needed - path indices from compiled metadata locate the nodes directly.
2. **Event handlers** - the framework uses compiled metadata event entries (`e[]`) and element paths (`el[]`) to locate event targets and install delegated listeners on the shadow root.
3. **Conditional markers** - `<!--wc-->` / `<!--/wc-->` pairs delimit `<if>` blocks. The framework evaluates the condition, hydrates the inner content if active, and keeps `<!--wc-->` as a runtime anchor.
4. **Repeat markers** - `<!--wr-->` / `<!--/wr-->` wrap the entire `<for>` range; `<!--wi-->` marks each item boundary. The framework hydrates each item with a scoped variable, keeps `<!--wr-->` as the anchor, and strips `<!--wi-->` and `<!--/wr-->` markers.

After wiring, SSR-only markers (end comments, item boundaries) are removed. Start comments for conditionals and repeats are kept as runtime anchors for future DOM updates.

## Choosing a Hydration Plugin

WebUI supports two hydration frameworks, selected at build time:

```bash
webui build src --plugin=webui   # WebUI Framework (recommended)
webui build src --plugin=fast    # FAST-HTML
```

| | WebUI Framework (`--plugin=webui`) | FAST-HTML (`--plugin=fast`) |
|---|---|---|
| **Package** | `@microsoft/webui-framework` | `@microsoft/fast-html` + `@microsoft/fast-element` |
| **Base class** | `WebUIElement` | `RenderableFASTElement(FASTElement)` |
| **State seeding** | Automatic from SSR markers | Manual in `prepare()` |
| **Update model** | Targeted path-indexed | Full observable chain |
| **Best for** | SSR-first apps, minimal JS | Complex client interactivity |

**WebUI Framework** is the recommended path. State is restored automatically during the hydration walk - you write a component class, and the framework handles the rest.

**FAST-HTML** is an alternative for teams already invested in the [FAST](https://fast.design/) ecosystem. It requires manually reading state from the pre-rendered DOM in a `prepare()` method. See the [FAST-HTML README](https://github.com/microsoft/fast/blob/main/packages/fast-html/README.md) for details.

## Performance

WebUI tracks hydration timing through the [Performance API](https://developer.mozilla.org/en-US/docs/Web/API/Performance_API) and a custom completion event.

### Measuring total hydration time

```typescript
window.addEventListener('webui:hydration-complete', () => {
  const entry = performance.getEntriesByName('webui:hydrate:total', 'measure')[0];
  if (entry) {
    console.log(`Hydration complete in ${entry.duration.toFixed(1)}ms`);
  }
});
```

### Performance marks emitted

| Mark | Timing |
|------|--------|
| `webui:hydrate:total:start` | First component begins hydrating |
| `webui:hydrate:total:end` | Last component finishes hydrating |
| **Measure:** `webui:hydrate:total` | Total wall-clock hydration time |

The `webui:hydration-complete` event fires once after every component on the page has finished hydrating. Use it to gate post-hydration logic or report metrics.

### What makes hydration fast

- **Single DOM walk** - each shadow root is traversed exactly once, not per-binding.
- **Path-indexed updates** - after hydration, only bindings referencing a changed property re-evaluate. No diffing.
- **No re-render** - the DOM from SSR is reused in place. The framework never recreates it.
- **Islands architecture** - components without interactivity never load JavaScript at all.

## Template Syntax

Both plugins use the **same template syntax** - the difference is in the TypeScript component class, not the template:

```html
<h1>{{title}}</h1>
<button @click="{onClick()}">Click me</button>
<for each="item in items">
  <p>{{item.name}}</p>
</for>
<if condition="isVisible">
  <span>Shown</span>
</if>
```

Event binding, interpolation, conditionals, and loops are compiled identically regardless of the hydration plugin. For a complete syntax reference, see [Interactivity](/guide/concepts/interactivity).

## Learn More

- [Interactivity](/guide/concepts/interactivity) - Component authoring model (decorators, events, refs)
- [Plugins](/guide/concepts/plugins/) - How parser and handler plugins work
- [Performance](/guide/concepts/performance) - Optimization techniques beyond hydration
