---
name: testing
description: Test patterns, fixture structure, and when to use E2E vs unit tests.
---

# Testing

Use this skill when writing or modifying tests in this repository.

## Rust crate tests

Unit tests live alongside code in `#[cfg(test)]` modules. Integration tests go in each crate's `tests/` directory.

```bash
cargo test -p microsoft-webui-handler   # one crate
cargo test --workspace                   # all crates
cargo xtask test                         # via xtask (part of quality gate)
```

Every code change ships with tests:

| Change type | Requirement |
|-------------|-------------|
| New public API | At least one unit test per function/method |
| Bug fix | Regression test that fails without the fix |
| Performance change | Benchmark comparison (before/after) |
| Refactor | Existing tests pass unchanged |

Never remove, weaken, or `#[ignore]` an existing test unless explicitly asked.

## Shared test utilities (`webui-test-utils`)

The `webui-test-utils` crate provides common Rust test helpers, builders, and fixtures. Check it before writing new helpers - avoid duplicating across crates.

```toml
[dev-dependencies]
webui-test-utils = { path = "../webui-test-utils" }
```

## webui-framework E2E fixtures

The `packages/webui-framework` package uses Playwright for E2E tests. Each fixture is a mini WebUI app compiled and rendered by the real pipeline.

See `tests/fixtures/README.md` for the full reference.

### Fixture structure

```
tests/fixtures/<name>/
  src/
    index.html                 Page template (uses the component)
    <tag-name>/
      <tag-name>.html          Component template (real WebUI syntax)
      <tag-name>.css           Component CSS (optional)
  state.json                   Initial render state (all bound properties)
  element.ts                   Component class (NO template registration)
  <name>.spec.ts               Playwright tests
  webui.config.json            Build options override (optional)
```

### How it works

The test server (`fixture-render.ts`) auto-discovers fixtures with `src/index.html`, calls `@microsoft/webui` `build()` + `render()` to produce SSR HTML with template IIFEs, hydration markers, and inventory. The result is served at `/<name>/fixture.html`.

### Creating a new fixture

1. Create `fixtures/<name>/src/index.html` — page template:

```html
<!DOCTYPE html>
<html lang="en">
<head><meta charset="utf-8"><title>My Fixture</title></head>
<body>
  <test-widget label="Hello"></test-widget>
</body>
</html>
```

2. Create `fixtures/<name>/src/test-widget/test-widget.html` — component template:

```html
<span class="label">{{label}}</span>
<span class="count">{{count}}</span>
<button class="inc" @click="{increment()}">+</button>
```

3. Create `fixtures/<name>/state.json` — initial state:

```json
{ "label": "Hello", "count": 0 }
```

4. Create `fixtures/<name>/element.ts` — component class only:

```typescript
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '../../../src/index.js';

export class TestWidget extends WebUIElement {
  @attr label = 'Hello';
  @observable count = 0;

  increment(): void {
    this.count += 1;
  }
}

TestWidget.define('test-widget');
```

5. Create `fixtures/<name>/<name>.spec.ts` — Playwright tests:

```typescript
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('widget fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/<name>/fixture.html');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-widget');
      return el && (el as any).$ready === true;
    });
  });

  test('renders SSR content', async ({ page }) => {
    await expect(page.locator('test-widget .label')).toHaveText('Hello');
    await expect(page.locator('test-widget .count')).toHaveText('0');
  });

  test('updates on click', async ({ page }) => {
    await page.locator('test-widget .inc').click();
    await expect(page.locator('test-widget .count')).toHaveText('1');
  });
});
```

### Template syntax quick reference

| Feature | Syntax |
|---------|--------|
| Text binding | `{{propertyName}}` |
| Event binding | `@click="{handler()}"` or `@click="{handler(e)}"` |
| Boolean attribute | `?disabled="{{isDisabled}}"` |
| Dynamic attribute | `href="{{url}}"` |
| Mixed attribute | `href="/items/{{id}}"` |
| Complex property | `:items="{{items}}"` |
| Element ref | `w-ref="{myElement}"` |
| Conditional | `<if condition="show">...</if>` |
| Negation | `<if condition="!hidden">...</if>` |
| Comparison | `<if condition="count > 0">...</if>` |
| Compound | `<if condition="active && !busy">...</if>` |
| Loop | `<for each="item in items">...</for>` |
| Nested loop | `<for each="group in groups">...<for each="item in group.items">...</for></for>` |
| Shadow DOM | `<template shadowrootmode="open">...</template>` |
| Root event | `<template shadowrootmode="open" @click="{handler(e)}">` |
| Slot | `<slot></slot>` |

### Dynamic children pattern

Components only created via `document.createElement()` (not in any template) need a false `<if>` in the page template to make their template IIFEs available:

```html
<body>
  <my-host></my-host>
  <if condition="showChild"><my-child></my-child></if>
</body>
```

With `state.json`: `{ "showChild": false }`.

### Per-fixture build config

Create `webui.config.json` to override build options:

```json
{ "css": "module" }
```

### Light-DOM fixtures

The pipeline always produces shadow DOM. For light-DOM hydration tests, use manual template registration with `registerCompiledTemplate` from `@microsoft/webui-test-support` and a hand-written `fixture.html`. See `fixtures/light-dom/` for the pattern.

### Running framework E2E tests

```bash
cd packages/webui-framework
pnpm build                    # build the framework
pnpm test                     # unit + E2E tests
npx playwright test           # E2E only
npx playwright test tests/fixtures/<name>/  # one fixture
```

### Test support package (`@microsoft/webui-test-support`)

The `packages/webui-test-support` package provides:

- **`registerCompiledTemplate(name, meta)`** — register a raw `TemplateMeta` object (for manual/light-DOM fixtures)
- **`renderTemplateScript(name, meta)`** — render a template as an inline `<script>` tag
- **`renderFixtures({ fixturesRoot })`** — build+render all pipeline fixtures
- **`buildFixtureEntries({ ... })`** — bundle element.ts files via esbuild
- **`startFixtureServer({ ... })`** — start the HTTP fixture server

## When to use what

| Scenario | Tool |
|----------|------|
| Rust crate logic (parsing, rendering, state, expressions) | `#[test]` unit tests |
| Rust crate integration (full pipeline) | `tests/` integration tests |
| Rust performance | `cargo bench` with Criterion |
| Framework hydration, reactive updates, DOM behavior | Playwright E2E fixtures |
| Router navigation, lazy loading, chain diffing | Playwright E2E fixtures |
| Template metadata encoding/decoding | Node unit tests (`pnpm test:unit`) |
