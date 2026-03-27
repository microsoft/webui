
### todo-webui (WebUI Framework hydration)

```bash
# Install JS dependencies (esbuild, @microsoft/webui-framework)
pnpm install

# Build the protocol with WebUI parser plugin
cargo run -p microsoft-webui-cli -- build examples/app/todo-webui/src --out examples/app/todo-webui/dist --plugin=webui

# Or use the dev server with live rendering
cd examples/app/todo-webui
cargo run -p microsoft-webui-cli -- serve ./src --state ./data/state.json --plugin=webui --servedir ./dist --port 3006
```

### Using `--plugin=webui`

The `--plugin=webui` flag enables:

1. **Parser plugin (`WebUIParserPlugin`)** — During `webui build`:
   - Skips WebUI Framework runtime attributes (`@click`, `w-ref`, etc.)
   - Counts dynamic attribute bindings per element and emits `Plugin` protocol fragments
   - Tracks components and generates `<w-template name="...">` client template strings

2. **Handler plugin (`WebUIHydrationPlugin`)** — During rendering:
   - Wraps signals, for-loops, and if-conditions in `<!--w-b:start:INDEX:NAME-->` comment markers
   - Wraps for-loop items in `<!--w-r:start:INDEX-->` comment markers
   - Emits `data-w-b-*` / `data-w-c-*` attributes for element bindings
   - Manages per-component/per-item scope counters for binding indices

These markers enable `@microsoft/webui-framework`'s client-side hydration.