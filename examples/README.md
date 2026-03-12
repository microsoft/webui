# Examples

This directory contains runnable WebUI examples.

## Structure

- `app/` — source app examples (templates, assets, data)
- `integration/` — host-language integrations that load `protocol.bin` and render HTML

Current entries:

| Example | Description |
|---------|-------------|
| `app/hello-world` | Basic WebUI app with signals, for-loops, if-conditions |
| `app/todo-fast` | FAST-HTML hydration app with components, `@event` bindings, `f-ref`, and `<f-template>` injection |
| `integration/node` | Node.js integration via native addon (supports `--plugin=fast`) |
| `integration/rust` | Rust integration via `webui-handler` (supports `--plugin=fast`) |

## Quick Start

### hello-world (no plugin)

```bash
# Build the protocol
cargo run -p webui-cli -- build examples/app/hello-world/templates --out examples/app/hello-world/dist

# Render with Rust
cd examples/integration/rust
cargo run -- ../../app/hello-world/dist/protocol.bin ../../app/hello-world/data/state.json
```

### todo-fast (FAST-HTML hydration)

```bash
# Install JS dependencies (esbuild, @microsoft/fast-element, @microsoft/fast-html)
pnpm install

# Build the protocol with FAST parser plugin (emits hydration data + <f-template> wrappers)
cargo run -p webui-cli -- build examples/app/todo-fast/templates --out examples/app/todo-fast/dist --plugin=fast

# Bundle the client-side entry point with esbuild
cd examples/app/todo-fast
pnpm build:client

# Render with FAST hydration markers (Rust integration)
cd ../../integration/rust
cargo run -- ../../app/todo-fast/dist/protocol.bin ../../app/todo-fast/data/state.json --plugin=fast

# Or use the dev server with live rendering
cd ../../app/todo-fast
cargo run -p webui-cli -- start ./templates --state ./data/state.json --plugin=fast --servedir ./dist --port 3001
```

### Using `--plugin=fast`

The `--plugin=fast` flag enables two things:

1. **Parser plugin (`FastParserPlugin`)** — During `webui build`:
   - Skips FAST-specific runtime attributes (`@click`, `f-ref`, `f-slotted`, `f-children`)
   - Counts dynamic attribute bindings per element and emits `Plugin` protocol fragments
   - Tracks components and injects `<f-template name="...">` wrappers at `</body>` with FAST syntax conversion (`<if>`→`<f-when>`, `<for>`→`<f-repeat>`)

2. **Handler plugin (`FastHydrationPlugin`)** — During rendering:
   - Wraps signals, for-loops, and if-conditions in `<!--fe-b$$...-->` comment markers
   - Wraps for-loop items in `<!--fe-repeat$$...-->` comment markers
   - Emits `data-fe-b-*` / `data-fe-c-*` attributes for element bindings
   - Manages per-component/per-item scope counters for binding indices

These markers enable FAST-HTML's client-side hydration to efficiently locate and re-attach to server-rendered dynamic content.

## More Details

See integration-specific READMEs:

- [integration/node/README.md](integration/node/README.md)
- [integration/rust/README.md](integration/rust/README.md)
