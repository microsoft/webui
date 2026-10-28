# @microsoft/webui-delivery

Templates, styles, and fresh state for [WebUI](https://microsoft.github.io/webui)
navigation. Delivery never imports a renderer or starts routing on import.

```bash
npm install @microsoft/webui-delivery
```

| Entry | Use |
| --- | --- |
| `router.js` | Server-authoritative navigation and optional route policies |
| `resources.js` | Shared JSON/NDJSON transport and resource registration |

Lazy `<if lazy>` regions do not use this package: the build emits their
templates as static packs and `@microsoft/webui-framework/lazy.js` loads them.

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

## Transports

The request deadline defaults to 10 seconds, including a deferred state tail.
`ndjson(maxRecordBytes?)` defaults to 256 KiB per record. JSON-only applications
do not ship the decoder when bundled with tree-shaking.

See the [Routing guide](https://microsoft.github.io/webui/build/routing)
for HTML declarations, initialization ordering, server APIs, and cancellation.
