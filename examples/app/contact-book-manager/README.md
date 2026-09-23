# Contact Book Manager

A full-featured contact book manager built with **WebUI SSR** and WebUI Framework client hydration. Demonstrates Atomic Design component architecture, IndexedDB offline storage, client-side routing, and responsive layout - all rendered server-side with the `--plugin=webui` pipeline.

Only components with custom event handlers ship TypeScript. Declarative pages,
display atoms, and list/card components are HTML-only and are claimed by the
explicit HTML-only runtime imported by the app entrypoint.

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
cargo run -p contact-book-desktop --features source
```

The desktop example uses a frameless window (`titlebar: { "style": "none" }`).
WebUI owns the edge-to-edge header, drag region, and minimize, maximize/restore,
and close buttons. Search, navigation, and caption buttons remain interactive
inside the draggable header through `webui-no-drag`. Window actions use the
SDK's native window bridge, without enabling application IPC.

The shared `data/state.json` defaults to `"mode": "web"`. Both source and packaged
Rust launches supply `"mode": "desktop"` before rendering, including subsequent
route requests. Desktop mode keeps the app header in place while route content
scrolls; web mode has no caption buttons or reserved native titlebar space.
There is no user-agent detection or client-side mode switch after first paint.

The host also provides a themed pre-paint background, persisted window geometry,
and a close-request lifecycle handler. The desktop seed requires `contacts` and
`groups` arrays. The host keeps these
canonical collections in shared Rust storage and derives dashboard, favorites,
and group lists for each route. Browser seed fields `filteredContacts`,
`favoriteContacts`, and `recentContacts` are ignored on desktop; global settings
and theme tokens remain available to the renderer.

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

## Desktop performance

The package command builds an optimized, runtime-only app runner by default,
not just an optimized packaging CLI. Use `--debug` for a debug package.
Measure the packaged app through page hydration and include
WebKit's helper processes when reporting memory.

The focused release microbenchmarks cover seed preparation and macOS response
buffer handoff, not complete window startup:

```bash
cargo bench -p contact-book-desktop --bench desktop_state
cargo bench -p microsoft-webui-desktop --features native --bench macos_response
```
