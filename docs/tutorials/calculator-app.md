# Building a Calculator with WebUI Framework

This tutorial walks through the Calculator example — a multi-mode calculator that demonstrates several key WebUI Framework patterns:

1. Compose multiple custom elements (`calc-app`, `calc-display`, `calc-button`)
2. Use `@attr` and `@observable` for reactive state
3. Switch between standard and scientific modes at runtime
4. Handle keyboard input alongside button clicks
5. Hydrate server-rendered HTML with the WebUI plugin

## Project Structure

```
examples/app/calculator/
├── demo.toml                     # Demo shell metadata
├── package.json
├── playwright.config.ts          # E2E test config
├── src/
│   ├── index.html                # WebUI template with CSS custom properties
│   ├── index.ts                  # Hydration entry point
│   ├── calc-app/calc-app.ts      # Root component — mode switching, input dispatch
│   ├── calc-button/calc-button.ts# Individual button — emits `button-press` events
│   ├── calc-display/calc-display.ts # Display — expression + result
│   └── modes/
│       ├── engine.ts             # Shared state machine & mode registry
│       ├── standard.ts           # Standard mode layout & operators
│       └── scientific.ts         # Scientific mode (trig, log, constants)
└── tests/
    └── calculator.spec.ts        # Playwright visual regression tests
```

## Key Concepts

### Custom Elements with `WebUIElement`

Each component extends `WebUIElement` from `@microsoft/webui-framework` and calls `define()` to register itself:

```ts
import { WebUIElement, attr } from '@microsoft/webui-framework';

export class CalcButton extends WebUIElement {
  @attr label = '';
  @attr value = '';
  @attr btnType = '';

  onClick(): void {
    this.$emit('button-press', { value: this.value });
  }
}

CalcButton.define('calc-button');
```

- **`@attr`** — Binds a class property to an HTML attribute, keeping them in sync.
- **`$emit()`** — Dispatches a `CustomEvent` that parent components can listen for.
- **`define()`** — Registers the custom element tag name with the browser.

### Reactive State with `@observable`

The root `CalcApp` component uses `@observable` for the button list, which changes when the user switches modes:

```ts
@observable buttons: ButtonData[] = [];
```

When `buttons` is reassigned, the framework automatically re-renders the dependent template region.

### Mode Engine Pattern

Calculator logic is decoupled from the UI via a mode registry (`modes/engine.ts`). Each mode (standard, scientific) registers itself and provides:

- **`buttons`** — The grid layout definition
- **`columns`** — Number of grid columns
- **`processInput(input, state)`** — Pure function that takes user input and current state, returns new state

This keeps the `CalcApp` component thin — it only dispatches to whichever mode is active.

### Server-Side Rendering & Hydration

The `index.html` uses WebUI template syntax (`{{mode}}`, `{{displayValue}}`, etc.) so the server pre-renders the initial state. The client entry point (`index.ts`) imports the components, triggering automatic hydration:

```ts
window.addEventListener('webui:hydration-complete', logHydrationTiming);

// Side-effect imports — register custom elements and trigger hydration
import './calc-app/calc-app.js';
import './calc-display/calc-display.js';
import './calc-button/calc-button.js';
```

No manual hydration call is needed — importing a `WebUIElement` subclass that calls `define()` is enough.

## Running the Example

```bash
# From the repository root
cargo xtask dev calculator
```

This builds the WebUI protocol, bundles the TypeScript, and starts a dev server on `http://localhost:3002`.

## Running Tests

The calculator includes Playwright visual regression tests:

```bash
cd examples/app/calculator
npx playwright test
```

To update snapshots after intentional UI changes:

```bash
npx playwright test --update-snapshots
```
