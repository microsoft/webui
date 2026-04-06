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

The `packages/webui-framework` package uses Playwright for E2E tests. Each fixture is a self-contained mini-app.

### Fixture structure

```
tests/fixtures/<name>/
├── element.ts          # Component class + template metadata registration
├── fixture.html        # Light DOM SSR fixture (pre-rendered HTML)
├── fixture-shadow.html # Shadow DOM SSR fixture (with <template shadowrootmode>)
└── <name>.spec.ts      # Playwright test file
```

### element.ts pattern

```typescript
import { WebUIElement, attr, observable } from '../../../src/index.js';
import {
  bindEvent, bindText, dynamic, nodePath,
  registerCompiledTemplate, slot,
} from '@microsoft/webui-test-support';

// Register compiled template metadata (what the build tool would produce)
registerCompiledTemplate('test-counter', {
  h: '<span class="count"></span><button class="inc">+</button>',
  text: [
    bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('count')),
  ],
  events: [
    bindEvent('click', 'increment', false, nodePath(1)),
  ],
});

export class TestCounter extends WebUIElement {
  @observable count = 0;

  increment(): void {
    this.count += 1;
  }
}

TestCounter.define('test-counter');
```

### fixture.html pattern (light DOM)

```html
<!DOCTYPE html>
<html lang="en">
<head><meta charset="utf-8"><title>Counter Fixture</title></head>
<body>
  <test-counter><span class="count">0</span><button class="inc">+</button></test-counter>
  <script src="/dist/<name>/element.js"></script>
</body>
</html>
```

### fixture-shadow.html pattern (shadow DOM)

```html
<!DOCTYPE html>
<html lang="en">
<head><meta charset="utf-8"><title>Counter Fixture</title></head>
<body>
  <test-counter><template shadowrootmode="open"><span class="count">0</span><button class="inc">+</button></template></test-counter>
  <script>window.__webui_shadow=true;</script><script src="/dist/<name>/element.js"></script>
</body>
</html>
```

### spec.ts pattern

Always test both DOM modes:

```typescript
import { expect, test } from '@playwright/test';

for (const mode of ['light', 'shadow'] as const) {
test.describe(`counter fixture [${mode} DOM]`, () => {
  test.beforeEach(async ({ page }) => {
    const file = mode === 'light' ? 'fixture.html' : 'fixture-shadow.html';
    await page.goto(`/<name>/${file}`);
    await page.waitForSelector('test-counter');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-counter');
      return el && (el as any).$ready === true;
    });
  });

  test('renders SSR content', async ({ page }) => {
    await expect(page.locator('test-counter .count')).toHaveText('0');
  });

  test('updates on interaction', async ({ page }) => {
    await page.locator('test-counter .inc').click();
    await expect(page.locator('test-counter .count')).toHaveText('1');
  });
});
}
```

### Running framework E2E tests

```bash
cd packages/webui-framework
pnpm build                    # build the framework
pnpm build:e2e                # build the test server + fixtures
npx playwright test           # run all fixtures
npx playwright test tests/fixtures/<name>/<name>.spec.ts  # one fixture
```

### Test support package (`@microsoft/webui-test-support`)

The `packages/webui-test-support` package provides:

- **`registerCompiledTemplate(name, meta)`** - register template metadata (what the build tool produces)
- **Binding builders** - `bindText`, `bindAttr`, `bindBoolAttr`, `bindEvent`
- **Path builders** - `nodePath`, `slot`, `dynamic`
- **Condition builders** - `identifier`, `eq`, `when`, `repeat`
- **Fixture server** - `buildFixtureEntries`, `startFixtureServer` (auto-discovers fixtures, bundles with esbuild, serves with a static file server)

This package is private (`@microsoft/webui-test-support`) and shared between `webui-framework` and `webui-router` tests.

## When to use what

| Scenario | Tool |
|----------|------|
| Rust crate logic (parsing, rendering, state, expressions) | `#[test]` unit tests |
| Rust crate integration (full pipeline) | `tests/` integration tests |
| Rust performance | `cargo bench` with Criterion |
| Framework hydration, reactive updates, DOM behavior | Playwright E2E fixtures |
| Router navigation, lazy loading, chain diffing | Playwright E2E fixtures |
| Template metadata encoding/decoding | Node unit tests (`pnpm test:unit`) |
