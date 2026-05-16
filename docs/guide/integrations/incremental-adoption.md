# Incremental Adoption

The WebUI integration recipes assume the SSR pipeline is fully wired: `webui build` produces a `protocol.bin`, a handler plugin renders Declarative Shadow DOM with hydration markers, and the client uses `declarativeTemplate()` plus `enableHydration()` to bind to the pre-rendered content. That is the destination. It is rarely the starting point for an existing host.

This page covers the typical path *to* that destination: how to ship a FAST 3.x custom element behind a kill switch in three steps, where each step is a small, reversible PR that can be deployed independently. Hosts that already adopt the [Embedded fragment](./fragments) recipe in one shot can skip directly to Phase 1; hosts that need to land code before the build pipeline is in place start at Phase 0.

The running example is a `<my-citation-group>` element that renders a list of citation chips inside an existing app's response stream. Swap the tag name for whatever you are introducing.

## Phase 0 - Ship the element with a data-attribute bootstrap

Goal: get the FAST class loaded, registered, and rendering on the client without depending on `webui build`, `protocol.bin`, or the handler pipeline at all. This phase exists so that the element can be merged, code-reviewed, and exercised behind a feature flag while you separately wire up the SSR side.

The host emits a synchronous placeholder with state encoded in a `data-*` attribute. The class decodes that attribute in `connectedCallback` and uses an imperative FAST template to render the shadow root client-side.

```html
<!-- Server emits this, synchronously, from whatever stream / template engine
     already exists. No protocol.bin, no plugin, no webui crate at this stage. -->
<my-citation-group
  data-state="eyJjaXRhdGlvbnMiOlt7ImluZGV4IjoxLCJ0aXRsZSI6ImV4YW1wbGUifV19">
</my-citation-group>
```

```ts
// my-citation-group.ts
import { css, customElement, FASTElement, html, observable, repeat } from '@microsoft/fast-element';

interface CitationItem {
  readonly index: number;
  readonly title: string;
}

const template = html<MyCitationGroup>`
  <ol>
    ${repeat(
      (host) => host.citations,
      html<CitationItem>`<li>${(c) => c.index}. ${(c) => c.title}</li>`
    )}
  </ol>
`;

@customElement({ name: 'my-citation-group', template, styles: css`` })
export class MyCitationGroup extends FASTElement {
  @observable public citations: ReadonlyArray<CitationItem> = [];

  public override connectedCallback(): void {
    super.connectedCallback();
    const encoded = this.getAttribute('data-state');
    if (!encoded || this.citations.length > 0) return;
    try {
      const decoded = atob(encoded);
      const parsed = JSON.parse(decoded) as { citations?: CitationItem[] };
      if (Array.isArray(parsed.citations)) this.citations = parsed.citations;
    } catch {
      // Leave citations empty; host can reconcile later.
    }
  }
}
```

What Phase 0 buys you:

- The element ships to production behind a kill switch (feature flag, env var, query param).
- The host can be incrementally retrofitted to emit the placeholder anywhere it currently renders the equivalent server-side HTML.
- No SSR work, no build step, no handler plugin.

What Phase 0 does not give you:

- No first-paint content. Browsers see an empty `<my-citation-group>` element until JS executes; this is the same constraint any client-only custom element has.
- No SEO crawlability for the element's content (unchanged from before adoption if the element is replacing something that was already client-rendered).
- No hydration of pre-rendered shadow DOM, because there is no pre-rendered shadow DOM.

These trade-offs are why this is *Phase 0*, not the destination.

## Phase 1 - Add SSR with the fast-v3 plugin

Goal: keep the kill switch and the Phase 0 fallback, but when the switch is on, render the element server-side using WebUI's `fast-v3` plugin and ship DSD-bearing HTML to the client.

This phase introduces three things:

1. A wrapper template that names the element as a fragment entry.
2. A `webui build` step that produces `protocol.bin` alongside the host's deployable artifacts.
3. A handler call in the request path that swaps the Phase 0 placeholder for the rendered fragment.

### 1. The wrapper template

`<f-template>` blocks register as *components* (keyed by tag name), not as fragments. To render exactly one component standalone via the [Embedded fragment](./fragments) recipe, create a thin wrapper file that resolves the component reference:

```html
<!-- src/templates.html -->
<f-template name="my-citation-group">
  <template shadowrootmode="open">
    <ol>
      <f-repeat value="{{citation in citations}}">
        <li>{{citation.index}}. {{citation.title}}</li>
      </f-repeat>
    </ol>
  </template>
</f-template>
```

```html
<!-- src/my-citation-group.html - the wrapper, this becomes the entry -->
<my-citation-group></my-citation-group>
```

Build:

```bash
webui build src --plugin fast-v3 --out dist
```

The resulting `dist/protocol.bin` exposes `my-citation-group.html` as a fragment that, when rendered with state, materialises the `<my-citation-group>` element with shadow DOM populated.

### 2. The render call

In Rust, wire a long-lived handler and call it from the request path. (For a Node host, swap the equivalent calls in `@microsoft/webui`; the shape is the same.)

```rust
use std::sync::Arc;
use webui::{FastV3HydrationPlugin, RenderOptions, ResponseWriter, WebUIHandler, WebUIProtocol};

let protocol = Arc::new(WebUIProtocol::from_protobuf_file("dist/protocol.bin")?);
let handler = Arc::new(WebUIHandler::with_plugin(|| {
    Box::new(FastV3HydrationPlugin::new())
}));

// Per request:
let options = RenderOptions::new("my-citation-group.html", "/");
handler.handle(&protocol, &state, &options, &mut writer)?;
```

The handler is `Send + Sync`; share it across tasks via `Arc::clone`. See [Thread safety](./rust#thread-safety) for the shared-handler pattern.

### 3. Client-side switch

Update the client entry so FAST hydrates the pre-rendered DSD instead of running the imperative template:

```ts
// entry.ts
import { enableHydration } from '@microsoft/fast-element/hydration.js';

enableHydration({
  hydrationComplete() {
    console.log('hydration complete');
  },
});

void import('./my-citation-group.js');
```

Inside `my-citation-group.ts`, the registration changes to declarative form:

```ts
import { FASTElement, observable } from '@microsoft/fast-element';
import { declarativeTemplate } from '@microsoft/fast-element/declarative.js';
import { observerMap } from '@microsoft/fast-element/observer-map.js';

export class MyCitationGroup extends FASTElement {
  @observable public citations: ReadonlyArray<CitationItem> = [];
}

void MyCitationGroup.define(
  { name: 'my-citation-group', template: declarativeTemplate() },
  [observerMap()]
);
```

The `data-state` decoding logic and the imperative `html` template stay in the file for now, gated by `if (this.shadowRoot === null)`. Phase 1 keeps both paths so the kill switch can roll back instantly.

## Phase 2 - Remove the Phase 0 fallback

Goal: once Phase 1 has burned in across all production traffic, delete the `data-state` decoding path and the imperative `html\`...\`` template. The `<f-template>` block in `templates.html` becomes the single source of truth.

The diff is mostly subtraction:

```ts
// my-citation-group.ts - Phase 2
import { FASTElement, observable } from '@microsoft/fast-element';
import { declarativeTemplate } from '@microsoft/fast-element/declarative.js';
import { observerMap } from '@microsoft/fast-element/observer-map.js';

interface CitationItem {
  readonly index: number;
  readonly title: string;
}

export class MyCitationGroup extends FASTElement {
  @observable public citations: ReadonlyArray<CitationItem> = [];
}

void MyCitationGroup.define(
  { name: 'my-citation-group', template: declarativeTemplate() },
  [observerMap()]
);
```

What was removed:

- The imperative `html<MyCitationGroup>\`...\`` template.
- The `@customElement` decorator.
- The `connectedCallback` override and `data-state` decoding.
- The base64 / JSON.parse defensive code path.

What stayed:

- The state shape (`citations: ReadonlyArray<CitationItem>`).
- The `<f-template>` block in `templates.html`, which the `fast-v3` plugin still extracts at build time.
- The `enableHydration({...})` bootstrap in the entry file.

The kill switch in the host can also be removed at this point if every consumer has migrated.

## When each phase is the right end state

Most adopters land at Phase 2 and stop. But two earlier exits are valid:

- **Phase 0 is the end state** if the element is never expected to render on first paint and the surrounding host is already a JS-heavy SPA. The cost of standing up `webui build` and a Rust or Node handler exceeds the benefit. The data-attribute bootstrap is enough.
- **Phase 1 is the end state** if the host needs to retain the legacy fallback indefinitely (third-party embeds where the host cannot guarantee `webui build` ran, deployments that mix old and new clients). Carrying both paths is the cost of keeping the kill switch live forever.

## What this recipe deliberately does not cover

- **Routing inside the embedded element.** Phase 1 passes `"/"` as the request path; if your fragment uses `<route>` directives, see [Embedded fragments](./fragments#fragments-and-routing).
- **Module CSS dedup across fragments.** The host page is responsible for deduplicating any `<style type="module">` tags emitted by multiple WebUI renders. See [Embedded fragments](./fragments#what-you-dont-get).
- **End-to-end app conversion.** This recipe is for *one element at a time*. To convert a whole app, start with the [full-page Rust](./rust) or [Node](./node) recipe.
