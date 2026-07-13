# Hydration

WebUI separates initial SSR hydration from later browser rendering:

| Component files | Initial page | Browser use |
|-----------------|--------------|-------------|
| `user-card.html` + `user-card.ts` or `user-card.js` | Hydrates behavior immediately; only decorated fields are seeded from bootstrap state | Authored class owns events, lifecycle, decorators, and imperative APIs |
| `user-card.html` only | Remains dormant over the server-rendered DOM with no bootstrap state | The framework can activate the compiled template when state is actually written or the router creates it |

This boundary keeps server render dependencies out of startup state without
breaking soft navigation or requiring empty TypeScript classes.

## Dormant Scriptless Components

Scriptless components may use text bindings, attributes, `<if>`, and `<for>`.
The compiler retains their browser template metadata and marks it as
compiler-owned. When `@microsoft/webui-framework` is loaded, it registers a
minimal host for the tag.

For an existing SSR instance, that host:

- does not read `#webui-data.state`
- does not walk the SSR DOM
- does not install bindings during startup
- activates on `setState`, a compiled parent property write, or a later
  observed attribute change

Activation preserves server-rendered repeat items when the triggering write
does not include that repeat's state root. The repeat remains unsynchronized
until its root is explicitly supplied; supplying an empty array removes the
items. This lets a one-key parent update activate a dormant child without
discarding unrelated SSR content.

Client-created instances mount immediately from the cached template. This is
what lets an HTML-only route render from a JSON partial without an empty
same-named module.

An app that is entirely static after SSR does not need to load the framework.
An app that wants scriptless soft navigation or browser-applied template state
must import `@microsoft/webui-framework` once in its browser entry.

## Authored Components

Add a sibling module only when the component owns browser behavior:

```typescript
import { WebUIElement } from '@microsoft/webui-framework';

export class UserCard extends WebUIElement {
  // Events, lifecycle, decorators, or imperative APIs belong here.
}

UserCard.define('user-card');
```

For authored components, behavior hydration and state hydration are separate:

- `@observable` and `@attr` fields form the initial hydration keys.
- Template roots plus those decorated fields form the partial-navigation keys.

Template-only values do not enter initial state just because the component has
an event handler, `w-ref`, lifecycle method, or imperative API. Their rendered
values already exist in the trusted SSR DOM. An authored component with no
decorators can therefore wire its behavior while the page still emits
`"state":{}`.

Components using `@event` must be authored because the compiler needs a real
handler implementation. Do not add an empty class merely to make template
bindings or routing work.

## State Projection

WebUI uses two state surfaces:

| Surface | Keys | Purpose |
|---------|------|---------|
| Initial full HTML | Reachable authored `@observable` / `@attr` fields | Seed JavaScript-owned state only |
| Partial navigation | Destination template roots plus decorated fields | Create or update authored and scriptless route components |

For scriptless components, initial hydration keys are empty and navigation keys
contain only template roots such as `title` or `items`. Fully static scriptless
components have no navigation keys.

Inactive sibling routes are excluded from both request projections. Components
behind active-route conditionals and loops are included conservatively. If no
startup consumer exists, WebUI writes:

```json
{"state":{}}
```

Projection is a performance and payload boundary, not a secrecy boundary.
Never place credentials, private tokens, or other secrets in browser render
state.

## Routing

The router publishes templates from initial HTML and JSON partials. The
framework registers compiler-owned hosts for scriptless route tags, so those
routes use normal chain-diffed soft navigation. Route chain entries do not carry
a `client` flag.

Document navigation remains a safety fallback when neither authored code nor
the compiler-owned host runtime registers the destination component.
