# Contact Book Manager

A full-featured contact book manager built with **WebUI SSR** and WebUI Framework client hydration. Demonstrates Atomic Design component architecture, IndexedDB offline storage, client-side routing, and responsive layout - all rendered server-side with the `--plugin=webui` pipeline.

## Quick Start

```bash
# From the repository root:

# Install dependencies
pnpm install

# Run
pnpm start
```

Or use the xtask shortcut to run from anywhere in the workspace:

```bash
cargo xtask dev contact-book-manager
```

Then open [http://localhost:3003](http://localhost:3003).

## Rust desktop host

The Rust-first desktop host lives in `examples/app/contact-book-manager/desktop`.
It defines route state providers in Rust with `RouteStateRegistry::route(...)`
and loads packaged bundles with `DesktopRuntime::from_bundle_config(...)`
instead of relying on a static exported site:

```bash
pnpm --dir examples/app/contact-book-manager run build:deps
pnpm --dir examples/app/contact-book-manager run build:client
cargo run -p contact-book-desktop
```

## Desktop package smoke test

Build and launch a Contact Book desktop app in one command:

```bash
PACKAGES=/tmp/contact-book-packages

cd examples/app/contact-book-manager
cargo run -p microsoft-webui-cli -- desktop package . \
  --target macos-app \
  --out "$PACKAGES"

open "$PACKAGES/Contact-Book-Manager.app"
```

The `webuiDesktop` config in `package.json` tells the sidecar to run
`build:deps` and `build:client`, build the `contact-book-desktop` Rust runner,
stage non-generated assets from `dist`, build the bundle, and package the
runner-backed `.app`.

To inspect the packaged macOS app, enable Safari > Settings > Advanced > Show
features for web developers, then open Safari's Develop menu while the app is
running.
