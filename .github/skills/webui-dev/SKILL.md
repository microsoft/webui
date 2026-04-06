---
name: webui-dev
description: Build interactive WebUI apps with compiled-template hydration, template syntax, component patterns, and CLI usage.
---

# WebUI App Development

Use this skill when building or modifying WebUI applications.

## Critical rules (memorize these)

1. **Every template binding must exist in the server state JSON.** The server renders from JSON. Missing keys render empty.
2. **HTML, CSS, TypeScript are separate files.** No JSX. No CSS-in-JS. No JS in templates.
3. **The `<template>` tag is optional.** The build tool auto-injects it. Include it only for root host events (`@custom-event` on the shadow root).
4. **Components inside `<for>` loops do NOT inherit loop variables.** Pass data via attributes.
5. **No ternary in templates.** No function calls in bindings. No mixed `&&`/`||`.
6. **No `this.querySelector()` for reactive state.** Use `@observable` + template bindings.
7. **Decorators: `@attr` (HTML attribute), `@observable` (reactive state).** Both work in SSR.
8. **`@attr({ mode: 'boolean' })` for true/false.** Present = true, absent = false. Never use string `"false"`.
9. **Only interactive components need a `.ts` file.** Components with no event handlers or reactive state are SSR-only — just `.html` (and optional `.css`). No class, no decorators, no `.define()`.

## Quick reference

**Interactive component (needs .ts):**

```typescript
import { WebUIElement, attr, observable } from '@microsoft/webui-framework';

export class MyComponent extends WebUIElement {
  @attr label = '';
  @attr({ mode: 'boolean' }) disabled = false;
  @observable count = 0;
  @observable items: Item[] = [];
  inputEl!: HTMLInputElement;  // populated by w-ref="inputEl"

  increment(): void { this.count += 1; }
  onKeydown(e: KeyboardEvent): void { if (e.key === 'Enter') this.submit(); }
}
MyComponent.define('my-component');
```

**SSR-only component (no .ts needed):**

```
stat-card/
├── stat-card.html   ← Template only
└── stat-card.css    ← Optional styles
```

The server renders it. On SPA navigation, the router auto-registers it
if `elementBase: WebUIElement` is set in `Router.start()`.

```bash
webui build ./src --out ./dist --plugin=webui
webui serve ./src --state ./data/state.json --plugin=webui --watch
```

## Full reference

The complete guide covering all template syntax, CLI flags, patterns, anti-patterns, routing, and language integrations:

📖 **[docs/guide/ai.md](/docs/guide/ai.md)**

Read that file before generating any WebUI code.
