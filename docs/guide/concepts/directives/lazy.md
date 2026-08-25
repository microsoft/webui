# Lazy Component Policy

Component rendering and hydration policies are build-time attributes on the
root `<template>` in a component HTML file.

## Full Work Reduction

```html
<template
  w-render="lazy"
  w-reserve-block-size="18rem"
>
  <article>...</article>
</template>
```

This is the recommended policy for component types repeated below the initial
viewport. It combines:

- `content-visibility: auto` for browser-managed style, layout, paint, and raster
  deferral
- `contain-intrinsic-block-size: auto 18rem` to preserve scroll geometry and
  remember the measured size after rendering
- visibility-deferred hydration through the optional
  `@microsoft/webui-framework/lazy-hydration.js` coordinator

`w-reserve-block-size` is required with `w-render="lazy"`. Use a typical
rendered block size for one instance. The compiler accepts one non-negative CSS
length, including absolute, font-relative, viewport, and container-query units.
It rejects percentages, negative lengths, keywords such as `auto`, and
functions such as `calc()`.

## Hydration Only

```html
<template w-hydrate="lazy">
  <article>...</article>
</template>
```

This advanced policy leaves rendering unchanged and defers only JavaScript
hydration. Use it when `content-visibility` containment is unsafe for the
component's layout.

Do not pair `w-render="lazy"` with `w-hydrate="lazy"` because rendering
deferral already includes visibility-deferred hydration.

## Interaction Application

```html
<template w-hydrate="interaction">
  <nav>...</nav>
  <outlet />
</template>
```

This application-root policy keeps the trusted SSR DOM active while deferring
the authored component graph and router until interaction. The compiler marks
the rendered root. Compose
`@microsoft/webui-framework/interaction-hydration.js` with the
framework-agnostic `@microsoft/webui-router/preload.js` handle to prefetch one
hovered route partial, then activate components and router without refetching.

Use this only once in the active document. See
[Interaction-triggered hydration](/guide/concepts/hydration#interaction-triggered-application-hydration).

## Lazy Rendering Until Interaction

For one offscreen SSR island that may never be used, combine the rendering and
interaction policies:

```html
<template
  w-render="lazy"
  w-reserve-block-size="18rem"
  w-hydrate="interaction"
>
  <button @click="{open()}">Open</button>
</template>
```

The browser can skip offscreen style, layout, paint, and raster work while the
component module graph remains unloaded. When interaction starts the graph,
this boundary hydrates eagerly before click replay; it does not also wait for
the visibility coordinator.

The interaction marker still has document-wide singleton semantics. Do not use
this combination for repeated list items. For a visible application shell,
`content-visibility` provides no initial rendering benefit: put
`w-hydrate="interaction"` on the shell and `w-render="lazy"` on repeated
offscreen descendants instead.

## Per-Instance Overrides

```html
<!-- Keep rendering deferral, but hydrate immediately. -->
<product-card w-hydrate="eager"></product-card>

<!-- Disable rendering and hydration deferral. -->
<product-card w-render="eager"></product-card>
```

Only these exact, case-sensitive values are recognized.

## Build and Runtime Behavior

The policy wrapper is build-only. WebUI strips the policy attributes. A
component that authors `<template shadowrootmode="open">` keeps that wrapper as
its declarative shadow root; every other component is Light, so the wrapper is
unwrapped and its content renders directly in the host.

For the full policy, the build emits one deterministic, nonce-aware
`<style data-webui-render-policy>` in the document head. This lets
`content-visibility` apply before first layout rather than waiting for component
JavaScript. It also emits tree-scope-safe `:host(...)` and component-tag
selectors in the component stylesheet, so instances nested inside another
component's shadow root receive the policy without applying it to the parent.

The SSR subtree remains in the DOM, so text and accessibility semantics are
preserved. This policy does not defer HTML parsing, DOM node construction,
custom-element definition, or resource discovery.

See [Hydration](/guide/concepts/hydration#lazy-hydration) for coordinator
loading, browser fallback behavior, interaction activation, and image guidance.
