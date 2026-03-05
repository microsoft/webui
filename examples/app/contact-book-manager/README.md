# Contact Book Manager

A full-featured contact book manager built with **WebUI SSR** and **FAST-Element** client hydration. Demonstrates Atomic Design component architecture, IndexedDB offline storage, client-side routing, and responsive layout — all rendered server-side with the `--plugin=fast` pipeline.

## Views

The app implements 7 views, routed via the `page` attribute on the root `<cb-app>` element:

| View | Description |
|------|-------------|
| **Dashboard** | Stats row (total contacts, favorites, groups) + 5 most recent contacts |
| **All Contacts** | Searchable list of all contacts |
| **Favorites** | Filtered view of starred contacts |
| **Group View** | Contacts filtered by group (Work, Family, Friends, Other) |
| **Contact Detail** | Full contact profile with edit, favorite, and delete actions |
| **Add Contact** | Empty form to create a new contact |
| **Edit Contact** | Pre-filled form to modify an existing contact |

The app ships with **15 sample contacts** across **4 groups** (Work, Family, Friends, Other).

## Architecture

### Atomic Design

Components follow [Atomic Design](https://bradfrost.com/blog/post/atomic-web-design/) principles:

```
src/
├── cb-app/          # Root shell — state, routing, event delegation
├── atoms/           # 6 stateless presentational primitives
├── molecules/       # 4 composite atom groupings
└── organisms/       # 6 full feature sections
```

### SSR + Hydration

1. **Server** — The Rust `webui-cli` pre-renders HTML using `--plugin=fast`. Templates use `{{mustache}}` interpolation, `<if condition="...">`, and `<for each="...">` directives, evaluated against `data/state.json`.
2. **Client** — `index.ts` registers all 17 components with `templateOptions: 'defer-and-hydrate'`. Templates are NOT bundled into JS — they already exist in the DOM from SSR.
3. **Hydration** — Components with a `prepare()` method read initial state from server-rendered DOM (e.g., contacts from hidden `data-*` spans), then FAST's observation system takes over for reactivity.

### State Management

All state lives in the root `<cb-app>` component:

- **`@attr`** fields for HTML-reflected attributes (`page`, `searchQuery`, `activeGroup`, counts)
- **`@observable`** fields for internal arrays (`contacts`, `filteredContacts`, `favoriteContacts`, `selectedContact`, `groups`)
- **IndexedDB** (`ContactBookDB`) persists contacts client-side. On init, `prepare()` seeds IndexedDB from server data if needed; every mutation calls `saveToDB()` immediately.

### Event Handling

Child components communicate upward via **bubbling `CustomEvent`s** (`bubbles: true, composed: true`). The root `<cb-app>` listens on its `shadowRoot` in `connectedCallback()` — a delegation pattern.

```
cb-header       → 'search', 'add-contact'
cb-sidebar      → 'navigate'
cb-contact-card → 'select-contact'
cb-contact-detail → 'edit-contact', 'toggle-favorite', 'delete-contact', 'back'
cb-contact-form → 'form-save', 'form-cancel'
```

## Prerequisites

- **Rust toolchain** — see `rust-toolchain.toml` at the repo root
- **Node.js** (≥18) + **pnpm**

## Quick Start

```bash
# From the repository root:

# Install dependencies
pnpm install

# Build the SSR protocol binary
cargo run -p webui-cli -- build ./examples/app/contact-book-manager/src \
  --out ./examples/app/contact-book-manager/dist \
  --css external \
  --plugin=fast

# Bundle client JS
npx esbuild examples/app/contact-book-manager/src/index.ts \
  --bundle --outfile=examples/app/contact-book-manager/dist/index.js \
  --format=esm --sourcemap

# Start dev server
cargo run -p webui-cli -- start ./examples/app/contact-book-manager/src \
  --state ./examples/app/contact-book-manager/data/state.json \
  --plugin=fast \
  --servedir ./examples/app/contact-book-manager/dist \
  --port 3001
```

Or use the xtask shortcut (runs server + client watcher concurrently):

```bash
cargo xtask dev contact-book-manager
```

Then open [http://localhost:3001](http://localhost:3001).

## Project Structure

```
contact-book-manager/
├── package.json
├── tsconfig.json
├── data/
│   └── state.json                    # Pre-seeded state (15 contacts, 4 groups)
├── dist/                             # Build output
│   ├── protocol.bin                  # SSR binary
│   ├── index.js                      # Bundled client JS
│   └── cb-*.css                      # Per-component stylesheets
└── src/
    ├── index.html                    # Root HTML shell
    ├── index.ts                      # Entry — registers all 17 components
    ├── cb-app/                       # Root app component
    │   ├── cb-app.ts
    │   ├── cb-app.html
    │   └── cb-app.css
    ├── atoms/
    │   ├── cb-avatar/                # Circular initials avatar
    │   ├── cb-badge/                 # Colored group pill
    │   ├── cb-button/                # Multi-variant button
    │   ├── cb-empty-state/           # Placeholder for empty lists
    │   ├── cb-icon-button/           # Square icon-only button
    │   └── cb-input/                 # Styled text input
    ├── molecules/
    │   ├── cb-form-field/            # Label + input + error message
    │   ├── cb-nav-item/              # Sidebar navigation row
    │   ├── cb-search-bar/            # Search input with clear button
    │   └── cb-stat-card/             # Dashboard KPI card
    ├── organisms/
    │   ├── cb-contact-card/          # Compact contact row
    │   ├── cb-contact-detail/        # Full contact profile view
    │   ├── cb-contact-form/          # Add/edit contact form
    │   ├── cb-contact-list/          # Scrollable contact list
    │   ├── cb-header/                # Sticky top bar
    │   └── cb-sidebar/               # Left navigation panel
    └── electron/                     # Optional Electron wrapper
        ├── main.ts
        └── preload.ts
```

## Component Catalog

| Layer | Tag | Purpose |
|-------|-----|---------|
| **Root** | `<cb-app>` | Application shell — state, routing, event delegation, IndexedDB |
| **Atom** | `<cb-avatar>` | Circular avatar with initials and colored background (sm/md/lg) |
| **Atom** | `<cb-badge>` | Pill label with group color variants (work/family/friends/other) |
| **Atom** | `<cb-button>` | Button with variant (primary/secondary/danger/ghost) and size |
| **Atom** | `<cb-empty-state>` | Centered icon + message for empty content areas |
| **Atom** | `<cb-icon-button>` | Square icon-only button with optional danger hover |
| **Atom** | `<cb-input>` | Styled text input with placeholder, type, and name |
| **Molecule** | `<cb-form-field>` | Label + `<cb-input>` + optional error message |
| **Molecule** | `<cb-nav-item>` | Sidebar row with icon, label, count badge, and active state |
| **Molecule** | `<cb-search-bar>` | Search input with icon and conditional clear button |
| **Molecule** | `<cb-stat-card>` | Dashboard card with emoji icon, numeric value, and label |
| **Organism** | `<cb-contact-card>` | Compact contact row: avatar, name, star, email, phone, badge |
| **Organism** | `<cb-contact-detail>` | Full-page contact view with edit/favorite/delete actions |
| **Organism** | `<cb-contact-form>` | Two-column add/edit form with group selector and notes |
| **Organism** | `<cb-contact-list>` | Renders contact cards via `<for>` loop with empty state fallback |
| **Organism** | `<cb-header>` | Sticky top bar with title, search, and "Add Contact" button |
| **Organism** | `<cb-sidebar>` | Fixed nav panel with static items + dynamic group list |

## Key Design Decisions

### Use `connectedCallback` listeners, not template `@event` bindings

FAST-HTML hydration processes templates declaratively. Event bindings like `@click` are not supported in the SSR template syntax. Instead, components attach listeners manually in `connectedCallback()` and use a `listenersAttached` guard to prevent duplicates on reconnection.

### Use `dispatchEvent` instead of `$emit`

FAST-Element's `$emit` helper is not available during the hydration stage. Components use `this.dispatchEvent(new CustomEvent(...))` directly, with `bubbles: true` and `composed: true` to cross shadow DOM boundaries.

### Use `field!: type` for fields set in `prepare()`

Fields initialized by the `prepare()` lifecycle hook use TypeScript's definite assignment assertion (`!`) rather than default values. This avoids overwriting server-hydrated state with empty defaults.

### No nested custom elements in templates

Server-rendered templates avoid nesting custom elements inside other custom element templates to prevent hydration mismatches between the SSR output and the client's shadow DOM expectations.

### Use CSS `::before` for emoji, not inline emoji

Emoji characters in SSR templates can cause double-encoding issues during the server render pass. Components use CSS `::before` pseudo-elements with `content` properties for display emoji instead.

### `data-action` delegation for multi-button components

Components with multiple action buttons (e.g., `<cb-contact-detail>`) use a single `click` listener and identify the action via `data-action` attributes, dispatching the appropriate event in a switch/case block.

## Responsive Layout

- **Desktop (≥768px):** Fixed sidebar (260px) + scrollable main content area
- **Mobile (<768px):** Sidebar hidden, single-column layout, "Add Contact" button label hidden (icon only)

## Electron (Optional)

The `src/electron/` directory contains an optional desktop wrapper. It uses the `webui-node` native addon to render `protocol.bin` + `state.json` into HTML, then serves it via a custom `webui://` protocol scheme inside an Electron `BrowserWindow`.

To use it, build the native addon first:

```bash
cargo build -p webui-node --release
```

The Electron entry point is not part of the standard web build — it is an alternative deployment target that reuses the same SSR binary and client JS bundle.
