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
