# From React to BTR: How Edge Migrated Its UI

*This is Part 4 of our series on WebUI. [Part 1](./blog-why-we-rebuilt-web-rendering.md) covered why we built a new rendering model. [Part 2](./blog-inside-webui-technical-deep-dive.md) explained the engine internals. [Part 3](./blog-building-interactive-apps.md) showed how to write apps with it. This post is about how we use it at scale inside Microsoft Edge.*

---

## Why Edge Had to Move

Edge's internal pages — Settings, History, Downloads, Wallet, and more than a dozen others — started life as React applications. React was a reasonable choice at the time. It's productive, well-understood, and has a proven component model. Our engineers knew it, our tooling supported it, and we could hire for it. For a few years, it worked fine.

But the architecture had a structural problem we couldn't engineer around: **every page paid the full client-side rendering tax**.

The pipeline looked like this on every single page load:

1. Download the JavaScript bundle
2. Parse it
3. JIT-compile it
4. Execute React's initialization
5. Build the virtual DOM
6. Reconcile against the (empty) real DOM
7. Paint

This pipeline ran regardless of how much of the page was actually dynamic. A Settings page with thirty toggles and a privacy policy link? Full pipeline. A Downloads page showing a list of completed files? Full pipeline. Every time.

On a modern dev machine with an NVMe drive and 32 GB of RAM, this was fast enough that nobody complained. On the machines our users actually have — 4 GB of RAM, spinning disks, three-year-old CPUs — this pipeline could take one to two seconds before anything appeared on screen. Users would click a menu item, stare at a blank page, and wonder if something had broken.

The problem is worse than it sounds, because **the JavaScript bundle is a serializing blocker**. Nothing else can happen until the JS finishes. The browser can't paint a single pixel of UI content until the bundle downloads, parses, compiles through the JIT, and executes. On a low-end machine with 4 GB of RAM, the JIT compiler is competing for memory with the OS, other tabs, and the browser process itself. Garbage collection pauses add further unpredictability. A bundle that takes 200 ms to compile on a developer workstation can take 800 ms or more on these constrained devices — and that's *before* React even starts building the DOM. Every millisecond of that compilation is a millisecond the user spends staring at nothing.

The obvious question: why not use server-side rendering? In a traditional web app, you'd run Node.js on your server and pre-render the HTML. But Edge isn't a traditional web app. Edge's internal pages (`edge://settings`, `edge://history`, `edge://downloads`) run *inside the browser itself* — there's no remote server. These are local pages rendered by the browser's built-in WebUI infrastructure. We can't spin up a Node.js process on every user's machine just to render settings. That would mean shipping and maintaining a separate JavaScript runtime alongside the browser, with its own memory footprint, update cycle, and security surface. It's a non-starter.

What we *can* do is render on the C++ side. Edge already has a C++ backend that provides data to these pages — user preferences, download lists, browsing history. The question was whether we could get that C++ backend to also produce the HTML, eliminating JavaScript from the rendering pipeline entirely.

The insight that changed our approach was simple: most Edge UI pages are *read-heavy*. Settings is a list of toggles. History is a chronological list. Downloads is a table with status indicators. These pages are 95% static content with occasional user interactions. Running a full client-side rendering framework to display what is essentially a pre-known document is architectural overkill.

We needed a model that matched the actual shape of our UI: render the known content instantly, then attach interactivity to the parts that need it.

That model is BTR — Build Time Rendering. BTR is the name we use internally; the open-source project is [WebUI](https://github.com/microsoft/webui). Same engine, same protocol, same compiler — BTR is just what Edge engineers call it.

The concept is straightforward: compile the templates to a binary protocol at build time. At runtime, Edge's C++ backend feeds state into the pre-compiled protocol and produces complete HTML. The browser's built-in WebUI infrastructure delivers that HTML directly to the renderer — no network hop, no server process. Then the client hydrates only the interactive islands. The browser does what browsers are best at — parsing and painting HTML — while JavaScript handles only what JavaScript must handle: user interaction.

## What BTR Looks Like Inside Edge

BTR inside Edge combines three layers, each responsible for a distinct phase of the rendering lifecycle.

**Layer 1: C++ rendering via the WebUI protocol.** Edge's internal pages (like `edge://downloads` or `edge://settings`) are served through the browser's built-in WebUI infrastructure. When the user navigates to one of these pages, a C++ route handler generates the initial state (localized strings, layout configuration, download items) and feeds it through the pre-compiled WebUI protocol to produce complete HTML. This all happens in-process — there's no HTTP server, no network request, no separate runtime. The C++ code *is* the "server" in this architecture, running directly inside the browser process. The rendered HTML is handed to the renderer process ready to paint.

**Layer 2: Declarative Shadow DOM.** The HTML that arrives in the renderer includes pre-rendered shadow roots using the `<template shadowrootmode="open">` syntax. The browser can paint this HTML immediately, with full encapsulated styling, without executing a single line of JavaScript. What the user sees in those first few hundred milliseconds is real, styled, laid-out content — not a loading spinner or a blank page.

**Layer 3: FAST-HTML hydration.** Once the HTML is painted, client-side JavaScript walks the pre-rendered DOM, attaches event listeners, and binds reactive state. This is hydration, not rendering. The DOM already exists; the JS just makes it interactive.

Every BTR page in Edge follows a common base class pattern:

```typescript
import { RenderableFASTElement } from '@microsoft/fast-html';
import { FASTElement, attr, observable } from '@microsoft/fast-element';

export class BtrElement extends RenderableFASTElement(FASTElement) {
  @attr appearance: 'hub' | 'full-page' = 'full-page';
  @observable strings: Record<string, string> = {};

  async prepare(): Promise<void> {
    // Load initial state from server-generated module
    const state = await initialStateService.init({ name: 'strings' }, { name: 'layout' });
    this.strings = state.strings ?? {};
  }
}
```

`BtrElement` handles the boilerplate that every page shares: loading localized strings, reading layout configuration, and coordinating with the initial state service that bridges C++ data into JavaScript.

Concrete components extend this base. Here's a simplified download item:

```typescript
export class DownloadItem extends BtrElement {
  @attr id = '';
  @attr title = '';
  @attr state = '';

  onItemClicked(e: MouseEvent): void {
    // Handle download action
  }
}

DownloadItem.defineAsync({
  name: 'download-item',
  templateOptions: 'defer-and-hydrate'
});
```

```html
<template shadowrootmode="open" @click="{onItemClicked(e)}">
  <div class="download-item" state="{{state}}">
    <div class="title">{{title}}</div>
    <div class="actions">
      <button @click="{onOpen()}">{{strings.open}}</button>
    </div>
  </div>
</template>
```

The `templateOptions: 'defer-and-hydrate'` flag is key. It tells the runtime that this component's template was already rendered by the server. Don't re-render it — just walk the existing DOM and attach bindings.

Every Edge BTR page also has a standard entry point that configures hydration:

```typescript
import { TemplateElement } from '@microsoft/fast-html';

performance.mark('downloads-hydration-started');

import './downloads-hub-app.js';

TemplateElement.options({
  'downloads-hub-app': { observerMap: 'all' },
  'download-item': { observerMap: 'all' },
}).config({
  hydrationComplete() {
    performance.measure('downloads-hydration-completed', 'downloads-hydration-started');
  },
}).define({ name: 'f-template' });
```

This entry point does three things: registers the components that need hydration, configures which observables should be watched, and emits a performance mark when hydration finishes. That performance mark feeds directly into our telemetry pipeline, which we'll cover later.

## The Spectrum of Complexity

A common objection to pre-rendering architectures is that they only work for trivial pages. Our experience says otherwise. BTR handles a wide spectrum of complexity inside Edge, and we haven't hit a cliff where it stops working.

### Lightweight (mostly static)

Permission prompts — a few lines of text with Accept/Deny buttons. The IE Reactivation notice — a static message with a single call-to-action. These pages are almost entirely pre-rendered HTML. The client JavaScript is a handful of event listeners and nothing more. BTR is a natural fit, but honestly, any architecture would work here. The win is consistency: even trivial pages use the same component model and build pipeline as complex ones.

### Medium complexity (lists, search, conditional UI)

This is where most of our migrated pages live.

**Downloads** manages a list of items, each with its own state machine (in-progress, paused, complete, failed, dangerous). It supports search, filtering, and drag-and-drop. The list is rendered server-side using `<for>` directives, and individual item state changes are handled reactively after hydration.

**History** presents a chronological list with date grouping, search, and bulk selection for deletion. The initial list and date headers come from the server. Client-side code handles search input, incremental loading, and multi-select.

**Settings** has nested categories, toggle switches, dropdown menus, and conditional sections that appear or disappear based on feature flags and enterprise policies. The `<if>` directive handles conditional rendering, and `@observable` properties drive reactive updates when the user changes a toggle.

These pages have real interactivity. They're not brochureware. But they still benefit enormously from BTR because the *first render* — the thing the user sees when the page opens — is static. The list of downloads, the history entries, the settings categories: all of that is known data that the C++ side can provide instantly.

### Complex interactive (rich state, real-time updates)

**Wallet** manages financial data with real-time updates flowing through Mojo IPC, interactive forms with client-side validation, and complex conditional UI. This page has the most client-side code of any BTR feature, but it still benefits from instant first paint. The user sees their saved payment methods and addresses immediately. The JS that enables editing, adding, and real-time sync hydrates afterward.

The key point: **the architecture scales continuously**. There's no threshold where BTR stops working and you have to fall back to a client-rendered framework. As a page gets more interactive, the proportion of work done by hydration increases, but the server-rendered first paint always gives you a head start.

## React vs. BTR: What Actually Changed

For teams evaluating a similar migration, here's what the before and after actually looks like.

**Before (React):**

```tsx
// React: Everything runs client-side
import React, { useState, useEffect } from 'react';

function DownloadItem({ id, title, state }) {
  const [currentState, setCurrentState] = useState(state);

  return (
    <div className="download-item">
      <div className="title">{title}</div>
      <button onClick={() => handleOpen(id)}>Open</button>
    </div>
  );
}
```

**After (BTR):**

```typescript
// BTR: Server renders HTML, client hydrates
export class DownloadItem extends BtrElement {
  @attr id = '';
  @attr title = '';
  @attr state = '';
}

DownloadItem.defineAsync({
  name: 'download-item',
  templateOptions: 'defer-and-hydrate'
});
```

```html
<!-- Server-rendered, browser paints immediately -->
<template shadowrootmode="open">
  <div class="download-item">
    <div class="title">{{title}}</div>
    <button @click="{onOpen()}">Open</button>
  </div>
</template>
```

The surface differences are obvious: no JSX, no hooks, no `useState`. But the meaningful architectural changes run deeper:

**No React runtime.** There's no virtual DOM, no reconciliation algorithm, no fiber scheduler. The browser's built-in DOM *is* the component model. This isn't just a bundle size optimization — it removes an entire category of runtime overhead.

**HTML is the template.** BTR templates are HTML with binding expressions, not JSX that compiles to `createElement` calls. The template is the thing the browser paints. There's no intermediate representation.

**State flows from the server.** Initial state comes from C++ via the route handler protocol, not from a client-side fetch or a Redux store. The page doesn't need to make an API call, wait for a response, and then render. The data is already in the HTML.

**Bundle size drops significantly.** Removing React, ReactDOM, and whatever state management library was in use (we had pages using Redux, MobX, and plain context — sometimes all three) has a real impact. BTR components are plain classes with a small hydration runtime. The total framework cost is a fraction of what React required.

**First paint is instant.** This is the headline change. HTML arrives in the renderer fully formed. The browser starts painting immediately. It doesn't wait for JavaScript to download, parse, execute, and build a DOM. The user sees content, not a blank page.

We want to be clear about one thing: **this isn't a universal argument that React is bad.** React is excellent for highly interactive applications where most of the UI is dynamic and user-driven — collaborative editors, design tools, complex dashboards with real-time data. BTR is excellent for content-heavy applications where most of the UI is known at render time with interactive islands layered on top. Edge's internal pages fall squarely in the latter category. The right rendering model depends on the shape of your UI, not on framework popularity.

## The Performance Impact

We track performance across all BTR pages in Edge using browser histograms that record First Contentful Paint (FCP), Largest Contentful Paint (LCP), and hydration timing on every page load. These aren't lab measurements — they're sampled from real users on real hardware. Here's what we've seen.

**LCP improvement.** React-based pages using client-side rendering typically recorded LCP between 1,000 and 2,000 ms on mid-range devices (think: consumer laptops with 8 GB RAM, integrated graphics, SATA SSDs). After migration to BTR, these same pages consistently show LCP around 260 ms. The HTML is pre-rendered and arrives ready to paint. The browser doesn't wait for JavaScript to produce the DOM.

**Hydration overhead.** Hydration — the process of walking the pre-rendered DOM and attaching event listeners and reactive bindings — typically takes 50 to 200 ms depending on the number of components on the page. This is measured via `performance.mark` and `performance.measure` calls that feed into our `webui:hydration-complete` histograms. Importantly, hydration happens *after* the user can already see and read the page. It's background work from the user's perspective.

**Bundle size reduction.** Removing React, ReactDOM, and associated state management libraries from a page's dependency graph results in a meaningful reduction in shipped JavaScript. BTR components are plain classes with attribute decorators and a small hydration runtime. The exact savings vary by page, but the direction is consistent across every migration.

One optimization worth calling out specifically: **during hydration, we suppress observable notifications to prevent re-rendering of `<if>` and `<for>` directives.** When the hydration walker encounters a `<for>` loop that the server already rendered into twenty list items, it needs to bind those items without triggering the `<for>` directive to re-evaluate and re-render the list. Without this suppression, we observed LCP spiking from approximately 260 ms to approximately 1,300 ms — the server-rendered HTML would get torn down and rebuilt by the client, defeating the entire purpose of pre-rendering. Getting this right was one of the harder engineering problems in the BTR runtime.

We emit performance telemetry for every BTR page, which lets us catch regressions quickly. If a code change causes hydration time to double on the Downloads page, we see it in the dashboards within hours and can investigate before it ships to users.

## Lessons Learned

After migrating twelve-plus features, we've accumulated some hard-won lessons.

**Parity first, performance second.** Our migration rule is simple: if a BTR page looks or behaves differently from the React version it replaced, that's a bug. Performance gains are a welcome side effect, but they never justify removing features or changing behavior. Users don't care about your architecture — they care that the page works the same way it did yesterday. Every migration starts with a detailed UI parity comparison and doesn't ship until parity is verified.

**Server state needs careful design.** The C++ route handler must produce *all* the initial state the page needs for first render. If a piece of state is missing, the page renders with an empty section or placeholder content. This forces you to think about your data model upfront — what does this page need to display on load? — in a way that React's fetch-on-mount pattern doesn't. That discipline is ultimately a good thing, but it's a different way of working that takes some adjustment.

**Hydration order matters.** Components hydrate in DOM order, top to bottom. If a parent component depends on state that a child component initializes during hydration, you have a sequencing problem. We handle this through the `prepare()` lifecycle method and careful use of async initialization, but it's a pattern that new contributors trip over. Document your hydration dependencies clearly.

**The web platform keeps getting better.** Every new platform primitive reduces the gap between "web app" and "native app." Adopted stylesheets let us share styles without duplication. The Navigation API gives us SPA-style routing without a framework. View transitions make page changes feel seamless. We're not fighting the platform — we're building on it, and the platform keeps improving underneath us.

## What's Next

We're continuing to migrate Edge features to BTR. Every migration follows the same playbook: verify UI parity, measure performance, ship with telemetry, and monitor.

The rendering engine that powers BTR inside Edge is open source. We've published it as [WebUI](https://github.com/microsoft/webui) — the same core template compiler, protocol format, and server-rendering engine that we use internally. BTR is the internal name; WebUI is the open-source project. If you're building content-heavy web applications and the rendering model described in this post sounds like it fits your use case, you can try it today:

```bash
npm install @microsoft/webui
```

We're also investing in making the web platform itself better for this kind of architecture. Declarative Shadow DOM improvements, CSS Module Scripts, and other standards work all contribute to a world where server-rendered web components are a first-class pattern, not a workaround.

If your team is doing a similar migration — moving from a client-side framework to a server-rendered or hybrid model — we'd genuinely like to hear about it. File issues on the repo, contribute if you're so inclined, or just share your experience. The problems we've solved inside Edge aren't unique to Edge, and the solutions shouldn't be either.

---

*This post is Part 4 of our WebUI series. Read [Part 1: Why We Rebuilt Web Rendering](./blog-why-we-rebuilt-web-rendering.md), [Part 2: Inside WebUI](./blog-inside-webui-technical-deep-dive.md), and [Part 3: Building Interactive Apps](./blog-building-interactive-apps.md).*
