---
name: webui-dev
description: Build interactive WebUI apps with compiled-template hydration, template syntax, component patterns, and CLI usage.
---

# WebUI App Development

Use this skill when building or modifying WebUI applications.

## Critical rules (memorize these)

1. **The template is the UI.** All structure lives in `.html`. Never `document.createElement`, `innerHTML`, `insertAdjacentHTML`, or `appendChild`. Show/hide with `<if>`, repeat with `<for>`. The only exception is mounting a lazily loaded component.
2. **CSS owns all styling and animation.** Never `el.style.x =`, `classList.toggle`, or `adoptedStyleSheets`. Bind `?data-active="{{expr}}"` and select `[data-active]` in CSS. Animate with `transition`, `@keyframes`, `@starting-style` - never `element.animate()` or a JS animation library.
3. **JavaScript is opt-in.** A component needs **no** `.ts` file unless it has an `@event`, a `w-ref` for an imperative API, a lifecycle hook, a fetch, or a public method API. `WebUIElement`, `@observable`, and `@attr` are optional - add them only when TypeScript reads/writes the value or it is public API. Otherwise the value belongs in the server state JSON.
4. **Use the web platform.** `<dialog>` over a div modal, `popover` over a JS dropdown, `<details>` over a JS accordion. Prefer `:has()`, `@container`, `color-mix()`, `light-dark()`, `content-visibility`.
5. **Every template binding must exist in the server state JSON.** Missing keys render empty, silently.
6. **HTML, CSS, TypeScript are separate files.** No JSX. No CSS-in-JS. No JS in templates.
7. **The `<template>` tag is optional.** The build tool auto-injects it. Include it only for root host events (`@custom-event` on the shadow root).
8. **Components inside `<for>` loops do NOT inherit loop variables.** Pass data via attributes.
9. **Text bindings are path lookups; comparisons belong in conditions.** `{{count}}` and `{{user.name}}` resolve a dotted state path - nothing else. `{{count > 0}}` is looked up as a key literally named `count > 0` and renders empty. Comparisons go in `<if condition="count > 0">` or `?active="{{section == 'guide'}}"`. Operators: `==`, `!=`, `<`, `>`, `<=`, `>=`, `&&`, `||`, `!`. **Forbidden everywhere:** ternary (`? :`), function calls, arithmetic (`items.length - 1` resolves as a path and silently fails - send a precomputed `lastIndex`), mixing `&&` with `||`, more than 5 logical operators.
10. **`w-ref` requires braces.** `w-ref="{inputEl}"`, never `w-ref="inputEl"` - non-braced fails the build with `invalid-w-ref`. Use it only for imperative APIs (focus, scroll, `showModal`), never to read state.
11. **`@attr({ mode: 'boolean' })` for true/false.** Present = true, absent = false. Never use string `"false"`.

## Quick reference

Most components need only HTML and CSS:

```html
<!-- user-card.html - no .ts file -->
<h2>{{user.name}}</h2>
<if condition="user.isAdmin"><span class="badge">Admin</span></if>
```

Add a class only when something interactive happens:

```typescript
import { WebUIElement, attr, observable } from '@microsoft/webui-framework';

export class MyComponent extends WebUIElement {
  @attr label = '';                          // set by a parent template
  @attr({ mode: 'boolean' }) disabled = false;
  @observable count = 0;                     // mutated by increment()
  inputEl!: HTMLInputElement;                // populated by w-ref="{inputEl}"

  increment(): void { this.count += 1; }
  onKeydown(e: KeyboardEvent): void { if (e.key === 'Enter') this.submit(); }
}
MyComponent.define('my-component');
```

```bash
webui build ./src --out ./dist --plugin=webui
webui serve ./src --state ./data/state.json --plugin=webui --watch
```

## Full reference

The complete guide covering all template syntax, styling and animation rules, anti-patterns, routing, and a pre-flight checklist:

**[docs/ai/SKILL.md](../../../docs/ai/SKILL.md)**

Read that file before generating any WebUI code.
