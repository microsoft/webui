# `@microsoft/webui-framework`

Web Component runtime for WebUI applications. Use `WebUIElement`, `@observable`,
and `@attr` to add interactive behavior to server-rendered or client-created
components.

See the [Interactivity Guide](https://microsoft.github.io/webui/learn/interactivity)
for the complete authoring reference, or the [agent reference](https://microsoft.github.io/webui/ai.md)
for AI-assisted application development.

## Install

```bash
pnpm add @microsoft/webui-framework
```

Enable decorators in your TypeScript configuration:

```json
{
  "compilerOptions": {
    "experimentalDecorators": true,
    "useDefineForClassFields": false
  }
}
```

## Author a component

Place the component's HTML, optional CSS, and TypeScript in the same directory.
Import its TypeScript module from your application entry.

### `counter-card.ts`

```typescript
import { WebUIElement, attr, observable } from '@microsoft/webui-framework';

export class CounterCard extends WebUIElement {
  @attr label = 'Clicks';
  @observable count = 0;

  increment(): void {
    this.count += 1;
  }
}

CounterCard.define('counter-card');
```

### `counter-card.html`

```html
<p>{{label}}: {{count}}</p>
<button @click="{increment()}">Increment</button>
```

Start the runtime once in your application entry:

```typescript
import { startRuntime } from '@microsoft/webui-framework/runtime.js';
import './counter-card.js';

startRuntime();
```

Use the component in a page:

```html
<counter-card label="Taps"></counter-card>
```

Build templates with the WebUI plugin and bundle your application's browser
entry:

```bash
webui build ./src --out ./dist --plugin=webui
```

Seed initial reactive values in server state so the first render matches the
component. Add a TypeScript class only when a component needs event handlers,
lifecycle code, or imperative behavior. HTML-only components need no class.
See [Components](https://microsoft.github.io/webui/learn/components/)
and [State management](https://microsoft.github.io/webui/learn/state-management/).

## Component API

| API | Purpose |
|---|---|
| `@observable` | Reactive state read or changed by TypeScript |
| `@attr` | Reactive state reflected to an HTML attribute |
| `static define(tagName)` | Register the class as a custom element |
| `$emit(name, detail?)` | Dispatch a bubbling, composed custom event |
| `$whenRendered()` | Await pending DOM updates and affected WebUI descendants |
| `<property>Changed(oldValue, newValue)` | Respond to initial and subsequent reactive property values |
| `protected hydratedCallback()` | Run once when this component's DOM and refs are ready |

Use `hydratedCallback()` for initial DOM work, rather than assuming
`connectedCallback()` means hydration has finished. Use native
`disconnectedCallback()` for per-connection cleanup.

## Post-render DOM work

Reactive assignments request rendering automatically. Await completion when
subsequent code needs the updated DOM:

```typescript
this.showAction = true;
await this.$whenRendered();
this.actionButton.focus(); // w-ref="{actionButton}" on the conditional button
```

Call it on a connected, mounted component. It includes affected WebUI
descendants and activated lazy regions, not unrelated future work or paint.
Handle rejected completion requests and recheck focus intent after a wait. See
[Wait for affected rendering](https://microsoft.github.io/webui/learn/interactivity#wait-for-affected-rendering)
for the full contract and template example.

For `<if lazy>` regions, import the generated `lazy.webui.js` before
`startRuntime()`. Use `$prefetch(path?)` to warm a region before activation.
See [Lazy UI](https://microsoft.github.io/webui/learn/lazy).

## Application guides

- [Template directives](https://microsoft.github.io/webui/learn/directives/):
  conditions, keyed lists, events, and DOM refs.
- [Hydration](https://microsoft.github.io/webui/build/hydration):
  lazy and interaction hydration, streaming setup, and Trusted Types.
- [Routing](https://microsoft.github.io/webui/build/routing):
  navigation and route-scoped state.
- [Performance](https://microsoft.github.io/webui/build/performance):
  application budgets, batching, and measurement.
- [CLI reference](https://microsoft.github.io/webui/reference/cli/):
  build options, CSS delivery, and component assets.

For runnable applications, see `examples/app/todo-webui`,
`examples/app/contact-book-manager`, and `examples/app/commerce`.
