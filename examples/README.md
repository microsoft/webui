# Examples

This directory contains runnable WebUI examples.

## Structure

- `app/` — source app examples (templates, assets, data)
- `integration/` — host-language integrations that load `protocol.bin` and render HTML

Current entries:

| Example | Description |
|---------|-------------|
| `app/hello-world` | Basic WebUI app with signals, for-loops, if-conditions |
| `app/calculator` | Basic WebUI app with calculator that has custom views and events |
| `app/todo-fast` | @microsoft/fast-element 3.x hydration app with components, `@event` bindings, `f-ref`, and `<f-template>` injection |
| `app/commerce` | WebUI Framework hydration app with a Rust backend for commerce demo app, dozens of controls. |
| `app/routes` | Nested declaritive routing demo showing 4 level deep routes full server side and client handoff. |
| `integration/node` | Node.js integration via native addon (supports `--plugin=webui` and `--plugin=fast-v3`; deprecated @microsoft/fast-element 2.x compatibility uses `fast-v2` or `fast`) |
| `integration/rust` | Rust integration via `webui-handler` (supports `--plugin=webui` and `--plugin=fast-v3`; deprecated @microsoft/fast-element 2.x compatibility uses `fast-v2` or `fast`) |

## Quick Start

### hello-world (no plugin)

```bash
# Build the protocol
cargo run -p microsoft-webui-cli -- build examples/app/hello-world/templates --out examples/app/hello-world/dist

# Render with Rust
cd examples/integration/rust
cargo run -- ../../app/hello-world/dist/protocol.bin ../../app/hello-world/data/state.json
```

### todo-fast (@microsoft/fast-element 3.x hydration)

```bash
# Install JS dependencies (esbuild, @microsoft/fast-element)
pnpm install

# Build the protocol with the @microsoft/fast-element 3.x parser plugin (emits hydration data + <f-template> wrappers)
cargo run -p microsoft-webui-cli -- build examples/app/todo-fast/src --out examples/app/todo-fast/dist --plugin=fast-v3

# Bundle the client-side entry point with esbuild
cd examples/app/todo-fast
pnpm build

# Render with FAST hydration markers (Rust integration)
cd ../../integration/rust
cargo run -- ../../app/todo-fast/dist/protocol.bin ../../app/todo-fast/data/state.json --plugin=fast-v3

# Or use the dev server with live rendering
cd ../../app/todo-fast
cargo run -p microsoft-webui-cli -- serve ./src --state ./data/state.json --plugin=fast-v3 --servedir ./dist --port 3001
```

### Using `--plugin=fast-v3`

The `--plugin=fast-v3` flag enables two things:

1. **Parser plugin (`FastV3ParserPlugin`)** — During `webui build`:
   - Skips FAST-specific runtime attributes (`@click`, `f-ref`, `f-slotted`, `f-children`)
   - Counts dynamic attribute bindings per element and emits `Plugin` protocol fragments
   - Tracks components and injects `<f-template name="...">` wrappers at `</body>` with FAST syntax conversion (`<if>`→`<f-when>`, `<for>`→`<f-repeat>`)

2. **Handler plugin (`FastV3HydrationPlugin`)** — During rendering:
   - Wraps signals, for-loops, and if-conditions in `<!--fe:b-->` / `<!--fe:/b-->` comment markers
   - Wraps for-loop items in `<!--fe:r-->` / `<!--fe:/r-->` comment markers
   - Emits `data-fe="COUNT"` attributes for element bindings
   - Manages per-component/per-item scope counters for binding indices

These markers enable @microsoft/fast-element 3.x client-side hydration to efficiently locate and re-attach to server-rendered dynamic content. The FAST examples use `enableHydration()` and declarative templates from `@microsoft/fast-element` 3.x as their FAST runtime dependency.

Deprecated `--plugin=fast-v2` and `--plugin=fast` continue to emit legacy @microsoft/fast-element 2.x markers for compatibility; do not use them for @microsoft/fast-element 3.x examples.

## More Details

See integration-specific READMEs:

- [integration/node/README.md](integration/node/README.md)
- [integration/rust/README.md](integration/rust/README.md)
