# Installation

WebUI Framework can be installed and used with various environments and languages. This guide covers the most common installation methods.

## npm (Recommended)

The `@microsoft/webui` npm package is the primary way to use WebUI. It gives you:

- **`npx webui build`** — the CLI for building templates into protocols
- **`import { build, render } from '@microsoft/webui'`** — a programmatic API for Node.js
- **Native performance** via platform-specific binaries (no compilation required)

::: code-group
```bash [npm]
npm install @microsoft/webui
```

```bash [yarn]
yarn add @microsoft/webui
```

```bash [pnpm]
pnpm add @microsoft/webui
```
:::

### Configure package.json

A typical project setup:

```json
{
  "scripts": {
    "build": "webui build ./src --out ./dist",
    "dev": "webui serve ./src --state ./data/state.json --watch"
  },
  "dependencies": {
    "@microsoft/webui": "latest"
  }
}
```

Run the development server with `npm run dev` and build for production with `npm run build`.

### Cross-Platform Support

The npm package uses platform-specific optional dependencies to deliver native binaries. Supported platforms are installed automatically — no Rust toolchain required.

## Client-Side Router (Optional)

For single-page navigation with client-side route transitions, install the router package:

::: code-group
```bash [npm]
npm install @microsoft/webui-router
```

```bash [yarn]
yarn add @microsoft/webui-router
```

```bash [pnpm]
pnpm add @microsoft/webui-router
```
:::

The router is a separate package because it's only needed for apps with client-side navigation. Server-only apps that do full page loads on every request don't need it.

See the [Routing guide](/guide/concepts/routing) for setup and usage.

## Rust CLI

Rust users who only need the CLI can install it directly from crates.io:

```bash
cargo install microsoft-webui-cli
```

Then build your app:

```bash
webui build ./my-app --out ./dist
```

See the [CLI Reference](/guide/cli/) for full usage details.

## Rust Library

For Rust applications that need programmatic build or render capabilities, add the `webui` crate:

```toml
[dependencies]
webui = "*" # see https://crates.io/crates/webui for latest version
```

This gives you access to `webui::build()`, `webui::BuildOptions`, `webui::BuildResult`, and `webui::inspect()` for build-time operations, as well as `webui::WebUIHandler` for rendering.

See the [Rust Handler](/guide/concepts/handlers/rust) guide for API details and examples.