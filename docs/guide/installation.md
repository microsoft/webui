# Installation

WebUI Framework can be installed and used with various environments and languages. This guide covers the most common installation methods.

## NPM Installation

Install the WebUI Framework package:

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

Add the following scripts to your package.json file:

```json
{
  "scripts": {
    "start": "webui dev",
    "build": "webui build"
  }
}
```

This allows you to run the development server with `npm start` and build for production with `npm run build`.

## Rust Installation

### Build the CLI from Source

```bash
git clone https://github.com/microsoft/webui.git
cd webui
cargo build --release
```

The `webui` binary will be at `target/release/webui`. You can copy it to a directory on your `PATH`:

```bash
cp target/release/webui ~/.cargo/bin/
```

Then build your app:

```bash
webui build ./my-app --out ./dist
```

See the [CLI Reference](/guide/cli/) for full usage details.

### Add WebUI to your Cargo.toml

```toml
[dependencies]
webui = { git = "https://github.com/microsoft/webui" }
```

### Create a WebUI Project

Create a new Rust project with WebUI:

```bash
cargo new my-webui-project
cd my-webui-project
```

### Configure Cargo.toml for Development

Update your Cargo.toml to include development commands:

```toml
[package]
name = "my-webui-project"
version = "0.1.0"
edition = "2021"

[dependencies]
webui = { git = "https://github.com/microsoft/webui" }

[[bin]]
name = "dev"
path = "src/dev.rs"

[[bin]]
name = "build"
path = "src/build.rs"
```

This allows you to run:

```bash
# For development
cargo run --bin dev

# For production build
cargo run --bin build
```