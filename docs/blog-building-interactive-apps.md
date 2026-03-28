# Building Interactive Apps with WebUI

*This is Part 3 of our series on WebUI. [Part 1](./blog-why-we-rebuilt-web-rendering.md) covered why we built a compile-time SSR engine. [Part 2](./blog-inside-webui-technical-deep-dive.md) explained how the engine works — the protocol, the parser, the route handler, and the plugin system. This post is about you: how to write apps with WebUI.*

WebUI is open source at [github.com/microsoft/webui](https://github.com/microsoft/webui).

---

## The Web Platform Grew Up

When React launched in 2013, the web platform was missing critical primitives. There was no native component model. No scoped styles. No declarative way to render structured UI without writing imperative DOM manipulation code. Frameworks like React, Angular, and Vue filled those gaps — and they filled them well. React, in particular, deserves credit for popularizing component-driven architecture and making UI development feel like function composition.

But here's the thing: the platform caught up.

Between 2020 and 2025, browsers shipped a series of features that directly address the problems frameworks were invented to solve:

- **Web Components** — Custom elements with lifecycle callbacks, encapsulated behavior, and the ability to compose them like any other HTML element. They're registered once and usable everywhere.
- **Declarative Shadow DOM** — Shadow roots that can be expressed in HTML markup using `<template shadowrootmode="open">`. The browser creates them on parse, no JavaScript needed to display encapsulated content. This is the primitive that makes server-rendered Web Components practical.
- **CSS Containment and Container Queries** — Scoped layout reasoning without CSS-in-JS tooling. A component can respond to its own container's size, not just the viewport. Style isolation comes from the shadow DOM boundary itself.
- **Adopted Stylesheets and CSS Module Scripts** — Shared styles across multiple shadow roots without duplicating `<style>` blocks. You author a stylesheet once and adopt it into any shadow root programmatically.
- **Navigation API** — Client-side routing built into the browser. Intercept navigations, manage history entries, handle transitions — all without hash hacks or framework routers.

The question for web developers has shifted. It's no longer "which framework should we pick?" It's a more fundamental one: **do we still need a framework at all?**

Our answer with WebUI is: use the platform directly. Web Components are the component model. Declarative Shadow DOM is the rendering primitive. CSS does the styling. What you actually need on top of the platform is a thin compilation layer that makes authoring productive — templates with bindings, reactive state, and a build step that turns it all into a pre-rendered protocol binary. That's what WebUI provides. Not a framework that replaces the platform, but a tool that compiles down to it.

---

## Why Opinionated Beats Flexible

React's flexibility is genuinely useful. You can structure a React app in a dozen different ways, integrate with any state management library, adopt any rendering strategy, and mix patterns freely across a codebase. That flexibility is why React won — it adapts to almost any team and any problem.

But that same flexibility is also why so many React apps end up bloated.

There's no guard rail in React preventing you from shipping 500 KB of JavaScript for a settings page. Nothing stops you from wrapping a static heading in three layers of context providers. The framework trusts you completely, and in large teams with dozens of contributors, that trust gets exploited — not maliciously, but incrementally, one reasonable-looking PR at a time.

Here are common React antipatterns that WebUI structurally prevents:

**Over-rendering.** React re-renders entire component subtrees when state changes. A single `setState` call at the top of a tree can trigger reconciliation across hundreds of child components, even if the actual change affects one text node. WebUI uses targeted path-indexed updates: when a reactive property changes, only the specific DOM bindings that reference that property are touched. There's no virtual DOM diff. There's no reconciliation pass. The update cost is proportional to what actually changed, not to the size of the component tree.

**Bundle creep.** Every React component imports React itself, plus hooks, plus context, plus whatever state management library the team adopted that quarter. A "simple" component might pull in 40 KB of framework code before it renders a single pixel. WebUI components are plain classes with decorators. The runtime is minimal — there's no virtual DOM engine, no fiber scheduler, no hook resolution system.

**Client-side rendering of static content.** In a React app, even a static `<h1>` tag is created by JavaScript at runtime. The server sends a JS bundle, the browser parses and executes it, React builds a virtual DOM, diffs it, and then creates the actual DOM node. For content that never changes, every one of those steps is wasted work. In WebUI, static HTML is just HTML. It's pre-rendered into the protocol binary. The browser parses it and paints it. Zero JavaScript cost.

**Framework lock-in.** React components can't run outside React. You can't take a React component and drop it into a Svelte app, or use it in a plain HTML page without mounting a React root. WebUI components are standard Web Components. They work in any browser. They work inside React, inside Angular, inside Vue, or inside nothing at all. The component is the interop boundary, not the framework.

WebUI is opinionated by design. Templates are HTML files. JavaScript is opt-in, loaded only for interactive islands. The build step enforces separation between static content and dynamic behavior. You can't accidentally ship a page that requires JavaScript just to display text. The constraints aren't limitations — they're the architecture.

---

## Writing Your First Interactive Component

Let's build something real. Here's a todo app using the WebUI Framework (`@microsoft/webui-framework`) — a complete interactive component with reactive state, event handling, and a server-renderable template.

### The component class

```typescript
import { WebUIElement, attr, observable, volatile } from '@microsoft/webui-framework';

export class TodoApp extends WebUIElement {
  @attr title = '';
  @observable items: TodoItemData[] = [];

  @volatile get remainingCount(): number {
    return this.items.filter(i => i.state !== 'done').length;
  }

  addInput!: HTMLInputElement;

  onAddKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      const text = this.addInput.value.trim();
      if (!text) return;
      this.items = [...this.items, { id: String(Date.now()), title: text, state: 'pending' }];
      this.addInput.value = '';
    }
  }

  onToggleItem(e: CustomEvent<{ id: string }>): void {
    const item = this.items.find(i => i.id === e.detail.id);
    if (item) {
      item.state = item.state === 'done' ? 'pending' : 'done';
      this.items = [...this.items];
    }
  }
}

TodoApp.define('todo-app');
```

### The template (todo-app.html)

```html
<template shadowrootmode="open"
  @toggle-item="{onToggleItem(e)}"
>
  <h1>{{title}}</h1>
  <div class="add-form">
    <input placeholder="What needs to be done?" w-ref="addInput" @keydown="{onAddKeydown(e)}" />
    <button @click="{onAddClick()}">Add</button>
  </div>
  <div class="todo-list">
    <for each="item in items">
      <todo-item id="{{item.id}}" title="{{item.title}}" state="{{item.state}}"></todo-item>
    </for>
  </div>
  <div class="footer">
    <span>{{remainingCount}} items remaining</span>
  </div>
</template>
```

That's the entire component. No JSX transform. No build plugin chain. No hook dependencies to reason about. Let's walk through the key concepts.

**`@attr`** declares a property that reflects to and from an HTML attribute. Setting `<todo-app title="My Tasks">` in HTML sets the `title` property on the class instance. Changing the property from JavaScript updates the attribute. It's two-way binding with the DOM — the same mechanism that native elements like `<input>` use.

**`@observable`** marks a property as reactive. When `items` changes, the framework knows which DOM bindings depend on it and updates only those bindings. There's no full re-render. The `items` array is replaced immutably (spread into a new array) to signal a change — a pattern that's familiar from React, but without the reconciliation overhead.

**`@volatile`** marks a computed getter. `remainingCount` is re-evaluated every time any binding that references it is checked. It's derived state — the value depends on `items`, and the framework knows to re-read it when the template needs the current value.

**`w-ref="addInput"`** stores a DOM reference. When the template is hydrated, the framework finds the element with `w-ref="addInput"` and assigns it to `this.addInput` on the component instance. It's the equivalent of React's `useRef`, but without the hook boilerplate.

**`@keydown="{onAddKeydown(e)}"`** is an event binding. The framework attaches a `keydown` listener to that input element and calls the method on the component instance. The `(e)` passes the native event object. No synthetic events. No event pooling.

**`<for>` and `<if>`** are template directives. During SSR, the build step evaluates them against the initial state and produces real HTML. After hydration, they continue to work client-side — when `items` changes, the `<for>` directive adds or removes DOM nodes to match the new array.

---

## How Hydration Works

### The problem hydration solves

The server has already rendered HTML. The browser has already parsed it and painted it to the screen. The user can see the content. But it's static — clicking a button does nothing, typing in an input doesn't trigger any logic. The client needs to attach event listeners and reactive bindings to the existing DOM without tearing it down and rebuilding it. This process is called *hydration*: making static HTML interactive.

Hydration is where many SSR frameworks stumble. React's hydration, for example, re-runs the entire component tree on the client to verify that the server output matches what the client would render. If there's a mismatch, it throws warnings and sometimes re-renders from scratch. Next.js and Remix have improved this story, but the fundamental approach — re-executing component logic client-side — adds cost proportional to page complexity.

### WebUI's hydration lifecycle

WebUI takes a different approach. Here's what happens, step by step:

1. **Server renders HTML with Declarative Shadow DOM.** The build step produces complete HTML with `<template shadowrootmode="open">` blocks. The browser creates shadow roots during HTML parsing — no JavaScript involved. The page is visible and styled immediately.

2. **JavaScript loads and custom elements upgrade.** The browser sees `<todo-app>` in the DOM and, once the element definition is registered, triggers the upgrade lifecycle. The class constructor runs.

3. **The framework detects the existing shadow root.** Instead of calling `attachShadow()` (which would fail — the shadow root already exists), the framework finds the shadow root that the browser created from the declarative template.

4. **It walks the DOM once.** The framework scans for SSR markers — HTML comments like `<!--w-b:start:items-->`, attributes like `data-w-b-title`, and event markers like `data-ev`. Each marker tells the framework which DOM node is bound to which property or event handler. This walk is linear, proportional to the number of bindings, not the size of the DOM.

5. **State is seeded from the pre-rendered DOM.** The framework reads attribute values and text content from the existing HTML to populate the component's initial state. There's no separate JSON payload to download and parse. The HTML *is* the state transport.

6. **Markers are removed and bindings go live.** The SSR markers are cleaned up (they were only needed for the hydration walk). Event listeners are attached. Reactive bindings are connected. The component is now interactive.

### What the user experiences

Nothing. That's the point. The HTML was already visible from SSR. Hydration happens behind the scenes — the user never sees a loading spinner, a blank page, or a flash of unstyled content. The transition from "static" to "interactive" is imperceptible.

### Measuring hydration performance

WebUI emits performance entries during hydration. You can measure exactly how long each component takes:

- `webui:hydrate:<tag-name>:start` / `webui:hydrate:<tag-name>:end` — per-component timing
- `webui:hydrate:total` — aggregate hydration duration

Here's the hydration entry point for our todo app:

```typescript
// index.ts
import './todo-app/todo-app.js';
import './todo-item/todo-item.js';

window.addEventListener('webui:hydration-complete', () => {
  const total = performance.getEntriesByName('webui:hydrate:total', 'measure')[0];
  console.log(`Hydration complete in ${total?.duration.toFixed(1)}ms`);
});
```

### Islands architecture

Not every component needs to hydrate. A page might contain 20 Web Components, but only 3 of them are interactive — a search input, a cart panel, a carousel. The other 17 are purely presentational: headings, product cards, navigation links, footers.

WebUI only ships JavaScript for the interactive components. The presentational ones stay as plain HTML inside their declarative shadow roots. No JS is downloaded, parsed, or executed for them. This is the islands architecture pattern: the page is a sea of static HTML with small islands of interactivity, and the cost of JavaScript is proportional to the amount of interactivity — not the amount of content.

---

## FAST-HTML: An Alternative Hydration Path

WebUI supports two hydration frameworks. You choose at build time with `--plugin=fast` or `--plugin=webui`. Everything we've covered so far uses the WebUI Framework plugin. Let's look at the alternative: FAST-HTML.

FAST-HTML builds on [FAST Element](https://www.fast.design/), Microsoft's lightweight Web Component library. Instead of `WebUIElement`, your components extend `RenderableFASTElement(FASTElement)` and use `defineAsync` for registration:

```typescript
import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class TodoApp extends RenderableFASTElement(FASTElement) {
  @attr title = '';
  @observable items!: TodoItemData[];

  async prepare(): Promise<void> {
    // Reconstruct state from pre-rendered DOM
    const items: TodoItemData[] = [];
    for (const el of this.shadowRoot!.querySelectorAll('todo-item')) {
      items.push({
        id: el.getAttribute('id') || '',
        title: el.getAttribute('title') || '',
        state: el.getAttribute('state') || 'pending',
      });
    }
    this.items = items;
  }
}

TodoApp.defineAsync({ name: 'todo-app', templateOptions: 'defer-and-hydrate' });
```

The key difference is in state seeding. With the WebUI Framework, the hydration walk automatically reads SSR markers and populates component state. With FAST-HTML, you implement a `prepare()` method that manually reconstructs state by reading the existing DOM. This gives you more control — you can transform data, merge sources, or handle edge cases — but it also means you're writing more code.

Here's how the two frameworks compare:

| Aspect | WebUI Framework | FAST-HTML |
|--------|----------------|-----------|
| Base class | `WebUIElement` | `RenderableFASTElement(FASTElement)` |
| State seeding | Automatic (from SSR markers) | Manual (`prepare()` reads DOM) |
| Ref binding | `w-ref="name"` | `f-ref="{name}"` |
| Update model | Targeted path-indexed updates | Full observable chain |
| Best for | SSR-first, minimal JS footprint | Complex client interactivity |

Both frameworks produce the same pre-rendered HTML output. The difference is entirely on the client side — how hydration happens and how updates propagate after hydration. If your app is primarily server-rendered with small interactive islands, the WebUI Framework's automatic approach means less code. If you're building something with heavy client-side state management — real-time collaboration, drag-and-drop interfaces, complex form wizards — FAST-HTML's explicit `prepare()` pattern gives you the control you need.

You don't have to choose one for your entire app. Different components can use different frameworks. The build step handles both plugins and produces a single protocol binary regardless.

---

## Scaling Up: A Commerce App

A todo app demonstrates the concepts, but it doesn't answer the question every team asks: *does this scale?*

The WebUI open-source repo includes a full commerce demo that answers that question directly. It's a multi-page application with dozens of components, real routing, server-driven state, and client-side interactivity. Here's what it includes:

- **Product cards** with image galleries and view transitions between list and detail views
- **Search** with real-time filtering that updates the product grid as the user types
- **Shopping cart** with add, remove, and quantity adjustment — state persisted across navigations
- **Nested routing**: home → search results → product detail, with animated transitions
- **Server-driven state** with client-side navigation — the initial render comes from the protocol, subsequent navigations fetch new state without full page reloads

Here's the component hierarchy, simplified:

```
mp-app (shell)
├── mp-navbar + mp-search-bar
├── mp-category-nav
├── <outlet /> (route content)
│   ├── mp-page-home → mp-hero-grid
│   ├── mp-page-search → mp-product-grid → mp-product-card
│   └── mp-page-product → mp-carousel
└── mp-cart-panel
```

Each component is a standard Web Component with its own HTML template, its own scoped CSS (inside the shadow root), and an optional client-side class for interactivity. `mp-product-card` is purely presentational — no JavaScript. `mp-search-bar` and `mp-cart-panel` are interactive islands with event handlers and reactive state. `mp-carousel` has client-side logic for swipe gestures and keyboard navigation.

The entire application builds to a single protocol binary. Routes, templates, static state, and layout — all pre-compiled. The binary is what the server (or embedded host) uses to respond to requests. There's no template engine running at request time. There's no server-side JavaScript execution. The response is assembled from pre-compiled segments and injected state.

Client-side JavaScript only loads for the interactive components. When a user lands on the home page, they get the full rendered HTML immediately. The search bar hydrates. The cart panel hydrates. Everything else is static HTML that the browser painted from the initial response. The total JS payload for the home page is a fraction of what an equivalent React SPA would ship.

---

## Deploy Anywhere the Web Platform Runs

Because WebUI builds on web standards — Web Components, Declarative Shadow DOM, standard CSS, the Navigation API — the same application runs in multiple deployment targets without code changes.

**Browser tab.** The most straightforward deployment. Serve the protocol binary from any HTTP server. The browser receives HTML, paints it, hydrates the interactive islands. It's a standard web application.

**Progressive Web App.** Add a manifest and a service worker. The same WebUI app becomes installable, works offline (serving cached protocol responses), and integrates with the OS — taskbar icons, system notifications, share targets. The service worker caches the protocol binary and static assets; subsequent visits load entirely from cache.

**WebView2 desktop app.** Embed the WebUI app inside a native Windows application shell using WebView2. The protocol binary is served locally — no network involved. Web content feels native because the rendering path is fast enough that users can't distinguish it from platform-native UI.

**Electron or Tauri.** Cross-platform desktop apps. Electron provides a Chromium-based shell; Tauri provides a lighter-weight alternative using the system's native WebView. Either way, the WebUI app runs unchanged inside it.

**Mobile WebView.** Embed the same app in an iOS `WKWebView` or Android `WebView`. The protocol binary is bundled with the native app. The web content renders alongside native UI, sharing the same data layer through a bridge.

The deployment target is a configuration choice, not a rewrite. The same component code, the same templates, the same protocol binary. You switch between targets by changing how the protocol is served and what shell hosts the WebView.

This is the payoff of building on the web platform instead of abstracting over it. You don't trade portability for performance. A React app running in Electron requires shipping the entire React runtime, a bundler output, and a Node.js process. A WebUI app in Electron ships pre-rendered HTML and a few kilobytes of hydration JS. The same performance characteristics you get in a browser tab carry over to every other target.

---

## Get Started

Install WebUI and start a dev server in one command:

```bash
npm install @microsoft/webui
npx webui serve ./my-app --state ./data/state.json --plugin=webui --watch
```

The `--watch` flag enables hot reload — edit a template or component class, and the dev server recompiles and refreshes. The `--state` flag points to a JSON file with your initial data. The `--plugin` flag selects your hydration framework (`webui` or `fast`).

From there:

- **[Playground](https://github.com/nickhstr/nickhstr.github.io)** — Try WebUI in the browser without installing anything
- **[Documentation](https://github.com/nickhstr/nickhstr.github.io)** — API reference, guides, and architecture deep-dives
- **[Examples](https://github.com/microsoft/webui/tree/main/examples)**:
  - `todo-webui` — Todo app with the WebUI Framework (the one from this post)
  - `todo-fast` — Same todo app with FAST-HTML
  - `commerce` — The full commerce demo described above
  - `routes` — Multi-page app with nested routing and view transitions

We'd love to see what you build. File issues, open PRs, or just star the repo if the approach resonates.

**Next up in the series:** In [Part 4](./blog-from-react-to-btr.md), we'll show how Edge itself migrated from React to this architecture across dozens of internal features — the decision process, the migration strategy, the performance results, and the lessons we learned shipping compile-time SSR to hundreds of millions of users.
