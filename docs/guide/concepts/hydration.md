# Hydration

WebUI renders components reached by the initial request on the server.
JavaScript is optional:

| Component files | Browser behavior |
|-----------------|------------------|
| `user-card.html` + `user-card.ts` or `user-card.js` | The authored class owns events, lifecycle, reactive state, and imperative APIs |
| `user-card.html` only | The server-rendered HTML stays inactive unless later navigation or state changes require browser rendering |

This keeps first-page work small without requiring empty TypeScript classes.

## HTML-Only Components

HTML-only components can use bindings, attributes, `<if>`, and `<for>`. Their
initial server-rendered DOM needs no hydration work or browser state.

When the framework is loaded, it can later activate the compiled template for
soft navigation or a browser-applied state update. Client-created instances
mount immediately. Existing repeated content remains in place until its
collection is explicitly supplied; supplying an empty array removes it.

An app that remains static after SSR does not need the framework. An app that
wants HTML-only soft navigation or browser-applied template updates imports
`@microsoft/webui-framework` once in its browser entry.

## Authored Components

Add a sibling module only when the component owns browser behavior:

```typescript
import { WebUIElement } from '@microsoft/webui-framework';

export class UserCard extends WebUIElement {
  // Events, lifecycle, decorators, or imperative APIs belong here.
}

UserCard.define('user-card');
```

Only `@observable` and `@attr` fields are eligible for exact initial state
projection. Ordinary template values already exist in the rendered HTML and do
not need to enter browser state just because the component has an event handler,
`w-ref`, lifecycle method, or imperative API.

An authored component with no decorators can therefore wire its behavior
without adding any startup state.

Load authored component definitions with a non-async ES module script, or place
a classic script after the component markup it defines. This guarantees that
the component subtree exists before upgrade. WebUI then hydrates synchronously
inside `super.connectedCallback()`; when it returns, bindings, events, and
`w-ref` references are ready.

Components using `@event` must be authored because the compiler needs a real
handler implementation. Do not add an empty class merely to make template
bindings or routing work.

## Build-Time State Projection

Exact state projection is opt-in. Rust does not inspect JavaScript or
TypeScript. The application bundles its browser code first, and a bundler
adapter emits `webui-projection.json` from the same resolved graph and output
membership that produced the browser chunks.

The projection compiler contract is bundler-neutral. The
`@microsoft/webui/projection.js` subpath currently includes the supported
esbuild adapter:

```bash
npm install -D esbuild typescript
```

```js
// build-client.mjs
import * as esbuild from 'esbuild';
import { esbuildProjection } from '@microsoft/webui/projection.js';

await esbuild.build({
  entryPoints: ['src/index.ts'],
  outdir: 'dist',
  bundle: true,
  splitting: true,
  format: 'esm',
  plugins: [esbuildProjection()],
});
```

Run the client build once, then give its manifest to WebUI:

```bash
node build-client.mjs
webui build ./src \
  --plugin=webui \
  --projection-manifest ./dist/webui-projection.json \
  --out ./dist
```

The generated file has this shape (hashes abbreviated):

```json
{
  "schema": "webui.state-projection/v1",
  "producer": {
    "name": "@microsoft/webui/projection.js",
    "version": "0.0.18"
  },
  "adapter": {
    "name": "esbuild",
    "bundler": "esbuild@0.28.1"
  },
  "root": "..",
  "analysisHash": "sha256:...",
  "buildId": "sha256:...",
  "outputs": {
    "dist/index.js": "sha256:..."
  },
  "inputs": {
    "src/user-card.ts": "sha256:..."
  },
  "components": {
    "user-card": {
      "module": "src/user-card.ts",
      "outputs": ["dist/index.js"],
      "hydrationKeys": ["displayName", "selected"],
      "navigationKeys": ["displayName", "selected"]
    }
  }
}
```

Do not hand-author this file. It is a deterministic record of the completed
bundle and becomes stale as soon as a declared input or output changes.

The manifest records exact input hashes, emitted output hashes, code-split
membership, component ownership, and sorted `@observable` plus `@attr` property
names. WebUI validates those hashes and embeds only the resulting key surfaces
in `protocol.bin`. Runtime handlers do not load the manifest, TypeScript, or a
bundler.

Behavior is intentionally strict:

- With no manifest, the build remains correct and sends full state. Projection
  is disabled rather than guessed.
- Once any manifest is supplied, every scripted component compiled into the
  protocol must have exactly one entry. Missing coverage fails with
  `PROJ-B001`.
- Shared controls supplied through `--components` remain application-owned
  bundles. If they are external to the main bundle, build them separately and
  pass each manifest fragment with another `--projection-manifest`.
- Stale inputs or outputs fail the WebUI build. Re-run the client bundler before
  rebuilding the protocol.
- `@attr` entries use JavaScript property names. During hydration, an existing
  SSR host attribute wins; projected state seeds the property only when that
  attribute is absent.

The adapter runs inside the application's existing esbuild invocation. It does
not start a second bundler run, and it does not constrain chunking, dynamic
imports, external modules, or output naming.

Other bundlers are not coupled to esbuild. A Vite, Rollup, Rolldown, webpack,
Rspack, or other adapter can construct the exported `AdapterContext`, call
`compileProjection()`, and run the exported conformance suite. The official
package currently ships and supports the esbuild adapter.

## State Sent to the Browser

With validated projection manifests, the initial page includes only
`@observable` and `@attr` values needed by authored components on the active
route. Template values used only for server rendering stay out of browser
state. Without manifests, WebUI preserves full state for compatibility and
correctness.

Later soft navigations include the values needed to render the destination
components. Inactive sibling routes do not enlarge either payload. If the
initial page needs no client state, WebUI writes:

```json
{"state":{}}
```

State sent to the browser is client-visible. Never place credentials, private
tokens, or other secrets in it.

## Routing

The router and framework can mount HTML-only routes from compiled templates
without empty component classes. If the framework is not loaded and no authored
custom element owns the destination tag, navigation falls back to a full page
request.
