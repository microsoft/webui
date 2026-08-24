# WebUI Press

A static site generator powered by the [WebUI Framework](https://github.com/microsoft/webui). Markdown in, hydration-ready HTML out, with no Node.js server required.

[![microsoft-webui-press on crates.io](https://img.shields.io/badge/crate-microsoft--webui--press-orange)](https://github.com/microsoft/webui)

`webui-press` is what powers [microsoft.github.io/webui](https://microsoft.github.io/webui). It is the WebUI Framework eating its own dog food: every page in the site is rendered server-side by the same protocol-compiled engine that the framework ships to consumers.

---

## Why

Most documentation site generators are JavaScript first. They run a Node.js server, ship a virtual-DOM bundle to the client, and re-render everything in the browser. That's a lot of moving parts for what is essentially a folder of markdown files.

`webui-press` takes the opposite approach:

- **Single Rust binary.** No Node.js server, no build server, no JavaScript runtime on the server. Drop the binary into CI, run it, ship the `dist/` folder.
- **Pre-compiled templates.** Pages are compiled into the WebUI binary protocol once, then rendered with state. Repeat builds reuse the cached protocol.
- **Parallel everything.** Page rendering is parallelized with [rayon](https://docs.rs/rayon). Syntax highlighting reuses one preloaded `syntect` syntax set across threads. Markdown parsing is per-page and free of cross-page state.
- **Hydration that works on GitHub Pages.** The output is static, server-rendered
  HTML using Light DOM for unwrapped components and Declarative Shadow DOM for opted-in
  components. No JavaScript is required for first paint. Optional client-side
  hydration upgrades interactive components without re-rendering anything.
- **Custom Web Components in markdown.** Drop a component into `components/`,
  reference it from any `.md` file with normal HTML, and it is server-rendered.
  Page-specific scripts can opt into esbuild bundling when they need npm
  imports.

---

## Install

```bash
cargo install microsoft-webui-press
```

Or as a workspace dependency:

```toml
[dependencies]
microsoft-webui-press = "0.0.10"
```

The binary is named `webui-press`. If your docs site includes component `.ts` files or per-page bundled scripts, install esbuild in the docs project:

```bash
pnpm add -D esbuild
```

---

## Quick start

```bash
# 1. Create a docs folder
mkdir docs && cd docs

# 2. Drop a config and a markdown file
mkdir -p .webui-press
cat > .webui-press/config.json <<'EOF'
{
  "site": { "title": "My Project" },
  "basePath": "/",
  "contentDir": ".",
  "outDir": "./dist",
  "nav": [{ "text": "Guide", "link": "/guide/" }],
  "sidebar": [],
  "sidebarGroups": {
    "/guide/": [
      {
        "title": "Getting Started",
        "items": [
          { "text": "Introduction", "link": "/guide/" },
          { "text": "Install",      "link": "/guide/install" }
        ]
      }
    ]
  }
}
EOF

mkdir -p guide
cat > guide/index.md <<'EOF'
# Welcome

Hello from `webui-press`.
EOF

# 3. Build
webui-press build
# → dist/ ready to deploy
```

Deploy `dist/` to GitHub Pages, Netlify, S3, or any static host.

---

## Project layout

```
docs/                          # contentDir (anything you want)
├── .webui-press/
│   ├── config.json            # site + nav + sidebar + hero + custom pages
│   ├── theme.css              # optional theme overrides (design tokens)
│   ├── components/            # optional custom Web Components
│   │   └── my-widget/
│   │       ├── my-widget.html
│   │       ├── my-widget.css
│   │       └── my-widget.ts
│   ├── public/                # optional static asset passthrough
│   └── state/                 # optional state JSON for custom pages
│       └── playground.json
├── index.md                   # homepage (with `layout: home`)
├── guide/
│   ├── index.md
│   └── install.md
└── dist/                      # generated, gitignored
```

Every `.md` file under `contentDir` becomes a page automatically. The sidebar/nav config controls navigation, not discovery.

---

## CLI

```
webui-press build [OPTIONS]

Options:
  -c, --config <PATH>      Path to config.json [default: .webui-press/config.json]
  -t, --template <PATH>    Override the bundled template directory
  -h, --help               Print help
```

The build pipeline:

```
1. Parse config              → DocsConfig
2. Discover .md files        → walk(contentDir) for every .md
3. Render markdown           → comrak GFM + syntect highlighting
4. Render component DOM      → Light DOM or opted-in Declarative Shadow DOM
5. Build WebUI protocol      → compiled per page, cached templates
6. Write base + theme CSS    → docs.css and theme.css emitted to outDir
7. Render pages in parallel  → rayon + WebUI handler
8. Generate search index     → JSON for client-side fuzzy search
9. Copy public/              → static asset passthrough
10. Write 404.html
11. Bundle scripts           → one esbuild build for root + needed page entries
```

Typical build for a 30-page site: under half a second on a laptop.

---

## Frontmatter

YAML frontmatter at the top of any `.md` file:

```markdown
---
title: Custom Page Title
description: Used for <meta description> and OpenGraph
layout: doc
---

# Page heading
```

| Field         | Type   | Default                                              |
| ------------- | ------ | ---------------------------------------------------- |
| `title`       | string | First H1, falls back to sidebar text, then site name |
| `description` | string | Falls back to `site.description`                     |
| `layout`      | string | `doc` (see below)                                    |

### Layouts

| Value  | Renders                                                                                |
| ------ | -------------------------------------------------------------------------------------- |
| `doc`  | Default. Sidebar + main content + prev/next + footer.                                  |
| `home` | Hero block + features grid (config-driven). No sidebar.                                |
| `page` | Wide markdown, no sidebar/page-nav. Normal scrolling.                                  |
| `full` | Viewport-fill, no chrome. Designed for single-component pages (e.g. an interactive playground). |

Shadow DOM components can react to the layout via `:host-context([data-layout="full"])` selectors.

---

## Configuration reference

`config.json` lives in `.webui-press/`. Every field is optional except `site`, `basePath`, `contentDir`, `nav`, and `sidebar`.

```json
{
  "site": {
    "title": "My Project",
    "description": "Used as default <meta description>"
  },
  "basePath": "/my-project/",
  "contentDir": ".",
  "outDir": "./dist",
  "publicDir": "./.webui-press/public",
  "theme": "@my-org/design-tokens",
  "css": "./.webui-press/theme.css",
  "components": ["./.webui-press/components"],
  "bundler": {
    "target": "es2022",
    "external": [],
    "define": { "process.env.NODE_ENV": "\"production\"" },
    "alias": { "~": "./src" },
    "projectionManifests": ["../client/dist/webui-projection.json"]
  },

  "head": [
    { "tag": "link",   "attrs": { "rel": "icon", "href": "/favicon.ico" } },
    { "tag": "script", "attrs": { "src": "/analytics.js", "defer": "" } }
  ],

  "nav": [
    { "text": "Guide",      "link": "/guide/" },
    { "text": "Tutorials",  "link": "/tutorials/" },
    { "text": "AI",         "link": "/ai", "source": "ai/SKILL.md" },
    { "text": "GitHub",     "link": "https://github.com/me/proj" }
  ],

  "sidebar": [],
  "sidebarGroups": {
    "/guide/": [
      {
        "title": "Getting Started",
        "items": [
          { "text": "Introduction", "link": "/guide/" },
          {
            "text": "Concepts",
            "link": "/guide/concepts/",
            "items": [
              { "text": "Components", "link": "/guide/concepts/components" }
            ]
          }
        ]
      }
    ]
  },

  "hero": {
    "text": "Big bold statement.",
    "tagline": "One-line subtitle that explains it.",
    "manifesto": "Optional paragraph for a manifesto stripe.",
    "actions": [
      { "text": "Get Started", "link": "/guide/", "brand": true },
      { "text": "GitHub",      "link": "https://github.com/me/proj" }
    ],
    "features": [
      { "icon": "⚡", "title": "Fast",     "description": "Sub-second builds." },
      { "icon": "🔌", "title": "Pluggable","description": "Drop in components." }
    ]
  },

  "footer": {
    "html": "Released under the MIT License."
  },

  "stateFile": "./state/site.json",

  "customPages": {
    "/playground/": {
      "layout": "full",
      "html": "<my-playground></my-playground>",
      "stateFile": "./state/playground.json",
      "scriptFile": "./components/my-playground/my-playground.ts"
    }
  }
}
```

### How `nav` and `sidebar` work together

- **`nav`** is the top bar. Links can be internal (`/guide/`) or external (`https://...`).
- **`sidebar`** is the default sidebar shown on pages that don't match a `sidebarGroups` prefix.
- **`sidebarGroups`** maps URL prefixes to sidebar definitions. The longest matching prefix wins. A page at `/guide/concepts/` uses the `/guide/` sidebar.
- **`prev`/`next`** links at the bottom of a page are derived from the active sidebar's flat link order.

### Routing

URLs are derived from the filesystem. Every `.md` file under `contentDir` becomes
a page; `nav` and `sidebar` control navigation, not discovery.

| Source file | URL |
| ----------- | --- |
| `index.md` | `/` |
| `guide/install.md` | `/guide/install` |
| `guide/index.md` | `/guide` |
| `components/my-button/my-button.md` | `/components/my-button` |

A trailing `index`, or a filename that repeats its parent folder, collapses to
the folder.

When a filename is dictated by an outside convention and should not leak into
the URL, point a `nav` entry at the file with `source` (a path relative to
`contentDir`, using forward slashes):

```json
{ "text": "AI", "link": "/ai", "source": "ai/SKILL.md" }
```

That serves `ai/SKILL.md` at `/ai` instead of `/ai/SKILL`. This is what keeps a
page installable as an agent skill — the [agent-skill
spec](https://agentskills.io) requires the file be named `SKILL.md` — while
keeping its published docs URL stable.

### Shared state

Use top-level `state` or `stateFile` when every page needs the same project data:

```json
{
  "stateFile": "./state/site.json"
}
```

The JSON must be an object and is merged into every page's render state, so
components and Markdown-authored custom elements can bind to it directly:

```html
<span>{{release.version}}</span>
```

Shared state cannot override reserved docs keys such as `site`, `navigation`,
`sidebar`, `page`, `hero`, `footer`, `prev`, `next`, `pageData`,
`regions`, `headTags`, `tokens`, `label`, or `icon`. Global state is applied
first. Custom page state is applied afterward for that page, so non-reserved
custom page keys can override global keys.

The merged render state is embedded into each generated page's `#webui-data`
hydration block. Do not put secrets in `state` or `stateFile`, and keep shared
state small. Large global JSON files are duplicated into every output page; use
custom page state or static JSON assets for large page-specific datasets.

### Compile-time named regions

Templates expose stable extension points without requiring a full template fork:

```html
<webui-press-region
  name="home.afterHero"
  layout="home"
>
  <h2>After the hero</h2>
</webui-press-region>
```

Sites provide the content in `config.json`:

```json
{
  "regions": {
    "home.afterHero": {
      "htmlFile": "./regions/home-after-hero.html",
      "stateFile": "./state/home-summary.json"
    }
  }
}
```

Child markup is the default. A matching `regions` entry can replace it with
`html` or `htmlFile`; omit both to keep the default while adding `state`,
`stateFile`, or `scriptFile`. Use `html: ""` to clear it. `layout` limits the
marker to one page layout; omit it for every layout. Dotted names create nested
state:

```html
<project-summary :data="{{regions.home.afterHero}}"></project-summary>
```

Default and replacement components participate normally in SSR, CSS, projection,
and bundling. A self-closing unconfigured marker is empty; a configured but
undeclared name fails the build. State-bearing dotted names cannot overlap as
prefixes.

The bundled template exposes these stable regions:

| Region | Layout | Default |
| --- | --- | --- |
| `site.navigation` | all | Logo and site navigation |
| `site.announcement` | all | Empty announcement/banner slot |
| `home.hero` | `home` | Hero, actions, and manifesto |
| `home.afterHero` | `home` | Empty slot after the hero |
| `home.features` | `home` | Feature card grid |
| `home.footer` | `home` | Site footer |
| `doc.sidebar` | `doc` | Documentation sidebar |
| `doc.context` | `doc` | Mobile current-location context |
| `doc.beforeContent` | `doc` | Empty slot before the article |
| `doc.afterContent` | `doc` | Empty slot after the article |
| `doc.pageNavigation` | `doc` | Previous/next links |
| `doc.footer` | `doc` | Site footer |
| `page.beforeContent` | `page` | Empty slot before wide content |
| `page.afterContent` | `page` | Empty slot after wide content |
| `page.footer` | `page` | Wide page footer |
| `full.beforeContent` | `full` | Empty slot before viewport content |
| `full.afterContent` | `full` | Empty slot after viewport content |

`home.*` regions belong to the generated home page. A `customPages` entry with
`layout: "home"` keeps the existing non-home shell and therefore uses `doc.*`
regions.

### `head` injection

Every entry in `head[]` is rendered into `<head>` with attributes sorted alphabetically (deterministic output for reproducible builds). Use it for favicons, analytics tags, preloads, OpenGraph overrides, anything `<head>`-shaped.

### `bundler`

`webui-press` uses [esbuild](https://esbuild.github.io/) for client JavaScript. It runs one build with a shared root entry plus page entries only when page content needs extra scripts, so page-specific code stays local and shared imports are split into reusable chunks automatically.

| Field      | Type             | Effect                                                   |
| ---------- | ---------------- | -------------------------------------------------------- |
| `target`   | string           | JavaScript target passed to esbuild. |
| `external` | string array     | Package IDs to leave external. Aliased packages are always bundled. |
| `define`   | object           | Compile-time replacements such as `process.env.NODE_ENV`. |
| `alias`    | object           | Module ID aliases. Relative targets are resolved from `config.json`'s directory. |
| `projectionManifests` | string array | Projection manifests for separately built bundles, resolved from `config.json` and watched by `serve`. |

You usually do not need a `bundler` section. Add one only when you need a package externalized to a CDN, a compile-time define, or a local alias.
Configured projection manifests are validated and merged even when the site has no local JavaScript entries.

---

## Theme and CSS overrides

The bundled `docs.css` is built around CSS custom properties. For design-token packages, set `"theme"` to the same kind of value accepted by `webui serve --theme`: a local JSON file, an npm package that exports `tokens.json`, or a package subpath.

```json
{ "theme": "@my-org/design-tokens" }
```

During each page build, webui-press reads the WebUI protocol token inventory emitted by the parser, resolves only those used tokens from the configured theme package, and injects the resolved CSS declarations into render state as `tokens.light`, `tokens.dark`, etc. Templates can inline them with raw CSS placeholders such as `/*{{{tokens.light}}}*/`.

Use `"css"` for site-specific chrome overrides that should stay outside the reusable theme package. The path is resolved relative to `config.json`'s directory:

```json
{ "css": "./.webui-press/theme.css" }
```

So the conventional layout is `.webui-press/config.json` + `.webui-press/theme.css` side by side, optionally with `"theme"` pointing at a shared token package.

Override the design tokens you care about:

```css
:root {
  --docs-color-brand: #6366f1;
  --docs-color-brand-hover: #818cf8;
  --docs-color-bg: #ffffff;
  --docs-color-bg-alt: #f8f9fa;
  --docs-color-text: #1a1a1a;
  --docs-color-text-2: #4a4a4a;
  --docs-color-text-3: #7a7a7a;
  --docs-color-border: #e5e7eb;
  --docs-font-sans: "Inter", system-ui, sans-serif;
  --docs-font-mono: "JetBrains Mono", monospace;
  --docs-max-width: 1280px;
  --docs-radius-s: 4px;
  --docs-radius-m: 6px;
}

[data-theme="dark"] {
  --docs-color-bg: #0a0a0a;
  --docs-color-text: #f5f5f5;
  /* ... */
}
```

Syntax-highlighting colors are also tokens (`--docs-hl-keyword`, `--docs-hl-string`, `--docs-hl-comment`, etc.) so light/dark themes flip automatically.

---

## Custom Web Components in markdown

Drop a component directory under `.webui-press/components/` (or any path listed in `config.components`):

```
.webui-press/components/my-callout/
├── my-callout.html
├── my-callout.css
└── my-callout.ts
```

Reference it from any `.md`:

```markdown
# Install

<my-callout type="warning">
  Make sure you have **Rust 1.93+** before continuing.
</my-callout>
```

Components are:

1. Compiled into the WebUI protocol at build time
2. Server-rendered as Light DOM when unwrapped or **Declarative Shadow DOM** when
   selected by a sole top-level `<template shadowrootmode="open">`
3. Auto-imported into the root script for template chrome or a page script when page content uses the component tag
4. Shared through esbuild chunks when multiple pages use the same component or dependency

Native slots require the component's open Shadow wrapper. Markdown inside those
slots is rendered as markdown, so you can mix prose and components freely.

See the [WebUI Framework component guide](https://microsoft.github.io/webui/guide/concepts/components) for authoring details.

### Per-page scripts

Use a page script when a single markdown page needs browser behavior that should not ship on every page. Mark a module script with the boolean `bundle` attribute:

```markdown
# Example

<p id="status">Waiting...</p>

<script type="module" bundle>
import { customButton } from "custom";

customButton();
document.getElementById("status").textContent = "Bundled script loaded.";
</script>
```

You can also point at a file:

```markdown
<script type="module" bundle src="./scripts/example.ts"></script>
```

`src` paths are resolved relative to `config.json`'s directory (`.webui-press/` by convention), not relative to the markdown file. For example, if your config is `.webui-press/config.json`, use `src="./scripts/example.ts"` for `.webui-press/scripts/example.ts`.

Bundled script files must stay inside the docs project (`config.json`'s directory or `contentDir`) and use a JavaScript/TypeScript extension (`.js`, `.mjs`, `.jsx`, `.ts`, or `.tsx`). Relative imports inside bundled scripts are checked against the same project roots, and absolute filesystem imports are rejected. Package imports such as `custom/button.js` remain supported.

`webui-press` extracts those scripts before rendering and imports them into that page's virtual esbuild entry. Plain `<script>` tags without `bundle` pass through unchanged.

Local component scripts are discovered automatically. If a page contains `<live-preview>` and the component source has `live-preview.html` next to `live-preview.ts`, the generated page entry imports `live-preview.ts`. Component templates are scanned too, so local child components are included without adding duplicate imports.

Package custom elements stay explicit because tag names cannot reliably be mapped back to package exports. Add those package registrations to the page's bundled script:

```markdown
<live-preview>
  <webui-button appearance="button">Click Me</webui-button>
</live-preview>

<script type="module" bundle>
import "@microsoft/webui-components/button.js";
</script>
```

The template chrome uses a shared root script (`index.js`). Pages with no page-specific component scripts or bundled imports load only that root script. Pages with identical page-specific import sets reuse the same generated import group instead of emitting duplicate wrappers.

All root and page entries are bundled in one esbuild build. If ten pages import the same package or local component runtime, esbuild can emit that dependency once as a shared chunk. Import-only page wrappers are collapsed after bundling, so generated pages link directly to the chunks they need instead of loading a `page-N.js` file that only re-imports the same chunks.

Generated root and page entries also receive build-time `<link rel="modulepreload">` hints for their static-import closures. Hints preserve the bundler's size order and use the same cache-busting URLs as the generated scripts.

`webui-press build` minifies bundled JavaScript. `webui-press serve` skips minification for faster rebuilds during local development.

### Built-in components

`webui-press` ships with these components pre-registered for content use:

| Tag                         | Purpose                                                |
| --------------------------- | ------------------------------------------------------ |
| `<code-block>`              | Syntax-highlighted code with a copy button (auto-injected around code fences) |
| `<webui-blockquote>`        | Styled quotes / callouts                               |
| `<webui-press-tabs>`        | Tabbed content groups                                  |
| `<webui-press-tab>`         | Tab triggers                                           |
| `<webui-press-tab-panel>`   | Tab content panels                                     |

Plus shadow components used by the chrome itself: `<docs-site-navigation>`,
`<docs-sidebar-navigation>`, `<docs-search>`, and `<docs-theme-toggle>`.

---

## Markdown features

Powered by [comrak](https://github.com/kivikakk/comrak), GitHub-flavored markdown:

- Tables, task lists, autolinks, footnotes, strikethrough
- Header anchors auto-injected with descriptive accessible labels
- Relative links resolve from the Markdown source directory and remain valid under `basePath`
- Fenced code blocks wrapped in `<code-block>` (copy button + dual-theme highlighting)
- Raw HTML pass-through, including custom elements

The generated shell uses a native `<dialog>` inside `<docs-site-navigation>`,
native `<details>` disclosures inside `<docs-sidebar-navigation>`, and a
skip-to-content link. The active documentation branch starts expanded. Normal
documentation pages use the cross-document fade; navigation into or out of a
`full` custom page skips that transition so viewport-filling app surfaces and
their page-specific bundles paint immediately.

### Syntax highlighting

Code blocks are highlighted by [syntect](https://github.com/trishume/syntect) with semantic CSS classes (`hl-keyword`, `hl-string`, `hl-comment`, ...) instead of inline styles. This means:

- One CSS file controls light and dark themes
- Adding a theme is editing a few CSS variables
- No FOUC, no client-side runtime

Recognized language tags (with aliases):

| Code fence            | Maps to    |
| --------------------- | ---------- |
| `js`                  | JavaScript |
| `ts`, `typescript`, `tsx`, `jsx` | JavaScript (closest available) |
| `rust`                | Rust       |
| `python`              | Python     |
| `go`, `golang`        | Go         |
| `c`, `cpp`            | C / C++    |
| `cs`, `csharp`        | C#         |
| `html`                | HTML       |
| `css`                 | CSS        |
| `json`                | JSON       |
| `yaml`, `yml`         | YAML       |
| `bash`, `sh`, `shell`, `zsh` | Bash |
| `toml`                | INI (closest available) |

Anything else falls back to plain text (still escaped, still themed).

---

## Custom pages

For pages that are pure interactive components (a playground, a live editor, a configurator), declare them in `customPages`:

```json
{
  "customPages": {
    "/playground/": {
      "layout": "full",
      "html": "<docs-playground></docs-playground>",
      "stateFile": "./state/playground.json",
      "scriptFile": "./components/docs-playground/docs-playground.ts"
    }
  }
}
```

| Field       | Effect                                                                              |
| ----------- | ----------------------------------------------------------------------------------- |
| `html`      | Page body. Usually a single component tag.                                          |
| `layout`    | `doc`, `home`, `page`, `full` (see [Layouts](#layouts)).                            |
| `state`     | Inline JSON merged into the page's render state under `pageData`.                   |
| `stateFile` | Path to a JSON file, resolved relative to `config.json`'s directory (`.webui-press/`). Each unique file is read and parsed once and shared across pages. |
| `scriptFile` | Path to a TypeScript or JavaScript file, resolved relative to `config.json`'s directory. The file is imported into this page's generated esbuild entry and linked only from this page. |

`state` and `stateFile` are mutually exclusive. State files are cached so multiple pages can share one source of truth without re-parsing.

`scriptFile` is useful for full-page components such as playgrounds. Keep the script next to its component HTML and CSS:

```
.webui-press/components/docs-playground/
├── docs-playground.html
├── docs-playground.css
└── docs-playground.ts
```

---

## Search

Every build produces a `search-index.json` next to the output, indexing every page's title, headings, and body text. The bundled `<docs-search>` component renders an instant fuzzy-search palette over it, with no server required.

Hide search by overriding the component or removing it from the template.

---

## Hydration model

The output is fully renderable without JavaScript:

- Markdown → HTML at build time
- Components rendered server-side via the WebUI protocol
- Light DOM for unwrapped components, with Declarative Shadow DOM pre-expanded for opted-in components
- Component styles installed in compiler-defined order

When the browser loads the generated root/page scripts (deferred, after first
paint), the framework finds the existing Light or Shadow DOM and **upgrades** it
in place, with no re-render, flash, or virtual DOM. Event handlers and
observable state are bound to the already-painted DOM. Page scripts import only
the local component scripts and explicit bundled scripts needed by that page,
with shared dependencies split into reusable chunks.

This is the WebUI Framework's [`webui` plugin](https://microsoft.github.io/webui/guide/concepts/plugins/) at work, and it is what makes the site feel instant on slow connections.

---

## Performance notes

- **Parallel rendering.** Pages render concurrently via rayon. Build time scales with cores, not page count.
- **Single esbuild build.** Template chrome, component TypeScript, and page script entries are bundled together so shared dependencies are split once and reused.
- **Cached protocol.** The WebUI binary protocol is built once per run and reused across all pages.
- **Shared highlighter.** One `syntect::SyntaxSet` is loaded per build and cloned per worker, not per page.
- **No regex in core paths.** Markdown processing, link normalization, and DSD pre-expansion are deterministic scanners.
- **Buffer-first IO.** HTML output uses pre-sized `String` buffers and `push_str`, never `format!` in hot loops.
- **Allocation-aware.** Hot data structures use `BTreeMap` (sorted, deterministic) over `HashMap` (non-deterministic, larger). Sidebar resolution is `O(prefixes)`, not `O(pages × prefixes)`.
- **Dev server is full-rebuild on every change.** `webui-press serve` re-runs the entire build pipeline on every filesystem event rather than tracking per-file dependencies. This keeps the dev path simple, makes every refresh byte-identical to a `build`, and benchmarks at sub-second rebuilds for sites under a few hundred pages. The only state amortized across rebuilds is the `syntect` highlighter (~30–50 ms to load).

If your build slows down, profile with `cargo flamegraph -p microsoft-webui-press --bin webui-press -- build`, every hot path is fair game for further optimization.

---

## Deploying to GitHub Pages

```yaml
# .github/workflows/docs.yml
name: Deploy docs
on:
  push:
    branches: [main]
permissions:
  contents: read
  pages: write
  id-token: write
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions-rust-lang/setup-rust-toolchain@v1
      - run: cargo install microsoft-webui-press
      - run: cd docs && webui-press build
      - uses: actions/upload-pages-artifact@v3
        with:
          path: docs/dist
  deploy:
    needs: build
    runs-on: ubuntu-latest
    environment:
      name: github-pages
      url: ${{ steps.deployment.outputs.page_url }}
    steps:
      - id: deployment
        uses: actions/deploy-pages@v4
```

Set `basePath` in `config.json` to `/<repo-name>/` so internal links work under the GitHub Pages subpath.

---

## Status

`webui-press` is the production builder for the WebUI Framework site. It is stable enough to ship a real documentation site to GitHub Pages today; it is not yet stable enough to promise no breaking changes between `0.0.x` versions. Pin a version, watch the changelog.

Issues, PRs, and feedback welcome at [github.com/microsoft/webui](https://github.com/microsoft/webui).

---

## License

MIT, Copyright (c) Microsoft Corporation.
