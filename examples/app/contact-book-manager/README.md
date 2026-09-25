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

The desktop example uses an overlay titlebar
(`titlebar: { "style": "overlay", "height": 48 }`) in both source and packaged
launches. The operating system owns the native caption buttons; the web header
does not render replacements or send window-action messages. WebUI owns the
edge-to-edge header and drag region. Search and Add Contact remain interactive
inside the draggable header through `webui-no-drag`.
The runner embeds the Contact Book icon for Windows executable and taskbar
display, including source launches. macOS packages use `desktop/icon.icns` for
the Dock and Finder icon; source launches pass that same file through
`DesktopShellConfig::icon_path`, so the Dock shows app artwork without an `.app`
bundle. The editable artwork is `desktop/icon.svg`.

On Windows, the 48-DIP header aligns with the Windows App SDK's tall native
caption region and requires the installed shared Windows App SDK runtime.
Search retains its 38px input height and Add Contact its 40px hit target.

The header reserves native control space with the SDK's
`--webui-titlebar-inset-start`, `--webui-titlebar-inset-end`, and
`--webui-titlebar-height` CSS variables. At narrow desktop widths the title
remains in the safe titlebar area while search and Add Contact move into a
second row below the native controls.

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

Both desktop build paths consume `dist/webui-projection.json` from
`build:client`. Source mode supplies it through Rust build options; packaging
uses `webuiDesktop.projectionManifests`. Keep the client build current before
launching or packaging. Navigation sends the fields required by the active
components, just as the browser host does; theme CSS remains available for full
document rendering without being repeated as unused navigation state.

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
