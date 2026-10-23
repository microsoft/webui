# @microsoft/webui-delivery

Templates, styles, and fresh state for [WebUI](https://microsoft.github.io/webui)
navigation and lazy UI. Delivery never imports a renderer or starts routing on
import.

```bash
npm install @microsoft/webui-delivery
```

| Entry | Use |
| --- | --- |
| `router.js` | Server-authoritative navigation and optional route policies |
| `lazy.js` | Preload resources and fetch fresh state for an owner-local `<if lazy>` |
| `resources.js` | Shared JSON/NDJSON transport and resource registration |

## Routing

```typescript
import { WebUIRouter } from '@microsoft/webui-delivery/router.js';

export const router = new WebUIRouter();
router.start();
```

The default requests JSON. Select and configure optional features once in
`start()`, not in the constructor:

```typescript
import {
  WebUIRouter, navigationCache, routeActions, routePreload,
  routeBoundaries, viewTransitions,
} from '@microsoft/webui-delivery/router.js';
import { ndjson } from '@microsoft/webui-delivery/resources.js';

const router = new WebUIRouter();
router.start({
  transport: ndjson(),
  cache: navigationCache({ maxEntries: 20, staleTime: 10_000 }),
  actions: routeActions(),
  preload: routePreload(),
  boundaries: routeBoundaries,
  transitions: viewTransitions,
  loaders: { 'settings-page': () => import('./settings-page.js') },
});
```

Selected listeners and transport implementations are ready at startup.
Application component modules load on demand. Calling `start()` again while
active is an error; use `destroy()` before restarting. Do not mutate loader maps
after startup. For pre-hydration intent loading, call a preload policy's
`prepare()` before activation and pass the same policy to `start()`.

## Lazy regions

```typescript
import { createLazyLoader } from '@microsoft/webui-delivery/lazy.js';
import { ndjson } from '@microsoft/webui-delivery/resources.js';

const panels = createLazyLoader({ transport: ndjson() });
await panels.ensureLoaded('app-shell', 'settings');
const state = await panels.fetchPartial('app-shell', 'settings', {
  url: '/api/settings',
  signal,
});
```

Preloading creates no UI and requests no application state. `fetchPartial()`
returns fresh state after resource readiness; the caller applies it to the
existing owner and changes its condition. Routing instead owns outlet
reconciliation and per-route state. Both share resource registration,
build-scoped inventories, cancellation, and response framing.

The request deadline defaults to 10 seconds, including a deferred state tail.
`ndjson(maxRecordBytes?)` defaults to 256 KiB per record. JSON-only applications
do not ship the decoder when bundled with tree-shaking.

See the [Routing guide](https://microsoft.github.io/webui/guide/concepts/routing)
and [Lazy UI guide](https://microsoft.github.io/webui/guide/concepts/lazy) for
HTML declarations, initialization ordering, server APIs, and cancellation.
