---
name: create-example
description: Create a runnable WebUI example with app structure, theme wiring, Playwright tests, demo metadata, and demo-shell registration.
---

# Checklist

1. Create the app folder with `src/`, `data/state.json` when SSR state is needed, `package.json`, `tsconfig.json`, and a concise `demo.toml`.
2. Use the shared theme for example UI: add `@microsoft/webui-examples-theme`, pass `--theme=@microsoft/webui-examples-theme`, and inject `/*{{{tokens.light}}}*/` plus `/*{{{tokens.dark}}}*/` in the entry template.
3. Keep `package.json` scripts consistent: `build`, `start:client`, `start:server`, `start`, `test`, and `test:update-snapshots` when Playwright applies.
4. Add Playwright coverage in `tests/` and a `playwright.config.ts` using the app's fixed dev port. Prefer behavior tests over screenshots unless the UI is the point.
5. Register the app in `xtask/src/e2e.rs` when it has Playwright tests. Use `pre_script: Some("build")` for custom build pipelines that the generic esbuild step cannot reproduce.
6. Add `demo.toml`, copy the app into `examples/demo/Dockerfile`, and list it in `examples/README.md`.
7. Run the focused app test, then `cargo xtask check`.
