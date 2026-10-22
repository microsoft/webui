# WebUI Press

WebUI Press is WebUI's native static site generator (SSG). It turns Markdown,
HTML templates, and Web Components into pre-rendered, hydration-ready static
output that can be deployed to GitHub Pages, a CDN, or any static host.

## Installation

Install the native CLI package:

```bash
npm install @microsoft/webui @microsoft/webui-press
```

The package installs the matching platform-specific binary automatically for
Windows, macOS, or Linux on x64 and arm64. After installation, run
`webui press build` or `webui press serve` directly without compiling the Rust
crate locally. The installed command starts the native Rust executable without
a Node wrapper; Node is used by npm during installation.

Use matching package versions and keep optional dependencies enabled.

## Dev-server shutdown

`webui press serve --shutdown-timeout 10` opts in to a ten-second shutdown grace
period. Without the flag, stopping waits for the active rebuild without a
deadline. Forced shutdown returns nonzero and can leave incomplete outputs.
See [bounded dev-server shutdown](/guide/cli/#bounded-dev-server-shutdown) for
second-stop behavior, stdin restrictions, and platform limits.

## Live reload

`webui press serve` rebuilds the site and reloads connected browsers when its
watched inputs change. Reading source files, including test discovery and
component imports, does not trigger a rebuild or browser reload.

## Content-only sites

The native `webui-press` binary supports the same display modes for static
builds and live development:

```bash
webui press build --show=content
webui press serve --show=content
```

The default is `all`. Set `"show": "content"` in `.webui-press/config.json`
to change that default; an explicit `--show=all` or `--show=content` wins,
including after serve reloads the config.

Content mode retains a complete document with metadata, base URL, themes,
semantic main/article content, SSR and hydration. Markdown, component examples,
API panels, custom-page HTML, and bundled page scripts remain. Home Markdown is
rendered too. All layouts and the 404 page use ordinary document scrolling,
without reserved sidebar columns, header offsets, or full-layout viewport fill.

Full mode remembers the theme control's light/dark selection and applies it to
native theme tokens and browser controls regardless of the OS preference.
Content mode has no shell theme control: it ignores the saved full-site choice
without changing it, follows OS light/dark changes through CSS, and works without
JavaScript. Forced-colors styles retain precedence over manual theme overrides.

Press header/navigation, sidebars/TOC, mobile navigation, previous/next links,
hero/features, footer, and template regions are not generated. Authored examples
are never removed based on their tag names. Region names/configuration are still
validated, but region state and scripts are inactive.

Even with `--template`, content mode uses the bundled shell-free scaffold and
content typography, not the full template's CSS or entry script. Configured
`head`, `css`, `theme`, `components`, and page scripts still apply. Put shared
assets there rather than in shell regions. Component discovery continues to
accept npm packages alongside local roots; browser registration imports can use
the existing `<script type="module" bundle>` syntax and bundler aliases.
Native npm discovery names components from `<tag>.html` files beneath the
package's `components/` directory by default. Package `webui.components` metadata
selects alternate roots, for example `["./src/components"]`. Folder names,
template exports, and CEM names do not determine native component names. Use
package module exports for browser registrations. See
[External components](/guide/concepts/components#external-component-sources).

## Page titles and canonical URLs

Markdown titles use frontmatter `title`, then the first H1, then the matching
sidebar label, then `site.title`. Frontmatter `description` overrides
`site.description`. Object-form custom pages can provide the same metadata:

```json
{
  "site": {
    "title": "My documentation",
    "description": "Guides and reference for the project.",
    "url": "https://example.com"
  },
  "basePath": "/docs/",
  "customPages": {
    "/playground": {
      "title": "Template playground",
      "description": "Try an HTML template with sample data.",
      "layout": "full",
      "html": "<my-playground></my-playground>"
    }
  }
}
```

Custom pages without these optional fields, including string-form custom pages,
retain the site title and description. Custom-page HTML replaces Markdown;
frontmatter from a Markdown file at the same URL does not supply metadata.

Both bundled display modes use `Page title | Site title` for the document and
Open Graph title. When the titles match, the site title appears only once.
An empty page title uses the site title; an empty site title leaves the page
title alone.

Optional `site.url` is an absolute HTTP(S) origin, such as
`https://example.com` or `http://localhost:4000`, with an optional trailing slash.
Do not include credentials, a deployment subdirectory, query, or fragment.
Use `basePath` for the deployment path. Press combines the origin with each
base-prefixed directory URL to emit a canonical link and `og:url`, including
in content-only mode. The example above gives the playground canonical URL
`https://example.com/docs/playground/`.
Omitting `site.url` emits neither tag. The generated 404 never emits canonical
metadata because it may serve many missing URLs.

Use the existing `head` configuration for site-owned share images, with a
public absolute image URL:

```json
{
  "head": [
    {
      "tag": "meta",
      "attrs": {
        "property": "og:image",
        "content": "https://example.com/docs/share.png"
      }
    }
  ]
}
```

## Page outlines

Enable the built-in "On this page" outline in `.webui-press/config.json`:

```json
{
  "outline": true
}
```

It is off by default. When enabled, `doc` and `page` layouts with at least two
anchored H2-H6 headings get a sticky desktop outline and a native mobile
disclosure. H1 is excluded. Home pages, `full` layouts, the generated 404, and
`--show=content` do not get the outline or its assets.

Override the site setting in Markdown frontmatter:

```yaml
---
title: Benchmark results
outline: false
---
```

`outline: true` can also enable an individual page when the site setting is off.
Object-form `customPages` accepts the same boolean override. Custom HTML
headings need explicit IDs. Use booleans, not quoted strings.

Links work without JavaScript. Browsers supporting `scroll-target-group` and
`:target-current` highlight the current section with an underline. A small
client helper keeps the selected link visible in long desktop outlines without
moving the article; reduced-motion preferences are respected. Outline CSS and
the helper load only on pages using the built-in outline.

The outline inherits the standard `--docs-*` theme tokens. Keep your colors and
fonts in the site's theme or custom CSS. Replace or clear `regions.page.outline`
to supply a different presentation; replacing its HTML omits the built-in
outline assets. When using `--template`, retain the bundled `page.outline`
region and the `docs-outline` component directory, containing its HTML, CSS,
and TypeScript.

## Page metadata for templates and regions

Press supplies these values in page render state in both display modes:

| Field | Value |
| --- | --- |
| `page.title` | Page title without the site suffix |
| `page.documentTitle` | Complete, deduplicated document title |
| `page.description` | Page-specific description or the site fallback |
| `page.path` | Base-prefixed directory URL, with a trailing slash |
| `page.canonicalUrl` | Absolute canonical URL when `site.url` is configured; otherwise `null` |
| `page.headings` | Ordered `{ text, anchor, level }` objects for content headings with IDs |
| `page.outline` | Whether the built-in outline is active for this page |
| `site.url` | Configured origin without a trailing slash, or `null` |

Headings are extracted at build time using the same text and anchors as search.
`text` is plain text with entities decoded and inline formatting removed;
`anchor` is the existing heading ID without `#`; `level` is a number from 1 to 6.
Press does not change heading IDs or create IDs for raw custom-page HTML.
Raw HTML headings need an explicit nonempty `id` to appear in this list.
Headings inside component templates or site regions are not included.
Full-mode home content is supplied by regions, so its heading list is empty;
content mode includes the home Markdown's headings.
The generated 404 has an empty heading list and `page.path` points to
`<basePath>404.html`.

A site can render an on-page contents list in `doc.beforeContent` without
browser JavaScript or a template fork:

```html
<if condition="page.headings.length > 1">
  <nav aria-label="On this page">
    <for each="heading in page.headings">
      <if condition="heading.level > 1">
        <a href="{{page.path}}#{{heading.anchor}}">{{heading.text}}</a>
      </if>
    </for>
  </nav>
</if>
```

Include `page.path` in fragment links: the bundled `<base>` points at the site
root, so a bare `#anchor` does not address the current page on a nested route.
Leave `outline` disabled when adding a separate site-owned contents list, or
replace `page.outline` rather than rendering a second outline. Content-only
mode still exposes heading metadata to authored content, but does not generate
shell regions.

## Markdown-local state

Put initial example data in the Markdown file that uses it, rather than in
the site's global config:

```markdown
---
title: Button examples
state:
  example:
    label: Save
    count: 2
    disabled: false
    items: [one, two]
---

# Button examples

<example-preview :example="{{example}}"></example-preview>
```

For this interactive example, the receiving component declares
`@observable example` and uses `example.label`, `example.count`, and
`example.disabled` inside its own template. Keep Markdown complex-property
bindings aligned with their top-level state key and receiving observable.
Mapping a nested value to a differently named observable at the Markdown entry
level does not currently preserve that value during initial hydration.

### JSON state files

Load page state from a JSON file:

```markdown
---
title: Button
stateFile: ./button.json
---
```

The file must contain a JSON object. Its top-level keys become page state:

```json
{
  "example": {
    "label": "Save",
    "count": 2
  }
}
```

This supplies the same `example` state as an inline frontmatter object.
`stateFile` accepts one path string, not a mapping or a list.

Paths resolve relative to the Markdown source, not `config.json`. Relative
`../` paths are supported; absolute and drive-relative paths are not.
Each file is read once per build, with a 16 MiB limit. `serve` watches included
files automatically, including generated files under `target`, and discovers
new dependencies when Markdown changes. Changes, atomic replacements, and
recreating a missing include or its parent directories trigger rebuilds.
No browser JSON fetch is needed.

`state` must be an inline JSON-compatible object. Nested objects, arrays,
strings, finite numbers, booleans, and `null` values are supported. Object
keys must be strings; quote numeric or boolean keys. Omit `state` or use `{}`
when no page-local data is needed. A top-level `null`, scalar, array, custom YAML tag,
non-finite number, or malformed YAML fails the build with the Markdown filename
and a corrective hint. Invalid, missing, or non-object JSON includes also fail
with the Markdown filename and a corrective hint.

Precedence, from lowest to highest: **global config `state` or `stateFile` →
Markdown `stateFile` → Markdown inline `state`**. Matching top-level keys are
replaced shallowly, for that page only. A local `example` value replaces the
entire global `example` object;
its nested fields are not deep-merged. Other global keys remain available,
and local data never flows into other Markdown pages or the generated 404.

Press owns these top-level keys: `site`, `navigation`, `sidebar`, `page`,
`hero`, `footer`, `prev`, `next`, `pageData`, `regions`, `headTags`, `tokens`,
`label`, and `icon`. Prototype-related keys `__proto__`, `prototype`, and
`constructor` are also reserved. Using any of them directly under Markdown `state`,
or at the top level of the included JSON object fails
the build. Put application fields beneath your own key, such as
`example.label`. Global and custom-page state retain their existing behavior:
reserved keys cannot replace canonical Press values.

The merged page state drives both initial SSR and the projected hydration
payload in `--show=all` and `--show=content`, including live rebuilds.
Treat all render state as browser-visible; never put secrets in it.
`customPages` still replaces Markdown at its URL entirely, so that Markdown's
frontmatter is not read. Custom-page state keeps its existing top-level merge
and `pageData` access; Markdown state does not populate `pageData`.

## Named regions

WebUI Press templates expose compile-time named regions that a site can keep,
replace, clear, or augment with state and browser code. Regions are resolved
before component discovery and protocol compilation, so their components receive
normal SSR, CSS, projection, and script bundling.

## Declare fallback content

A paired marker renders its child markup when the site has no matching
configuration:

```html
<webui-press-region name="home.afterHero" layout="home">
  <project-summary></project-summary>
</webui-press-region>
```

Use a self-closing marker for an empty insertion point:

```html
<webui-press-region name="site.announcement" />
```

`layout` is optional. Without it, the region is active on every layout. Use a
space-separated list such as `layout="doc page"` to share a region across
selected layouts; its markup, state, and scripts use the same scope.

## Configure a region

Add `regions` to `.webui-press/config.json`:

```json
{
  "regions": {
    "home.afterHero": {
      "htmlFile": "./regions/home-after-hero.html",
      "stateFile": "./state/home-summary.json",
      "scriptFile": "./scripts/home-summary.ts"
    }
  }
}
```

- `html` or `htmlFile` replaces the fallback markup. Set `html` to `""` to
  clear it.
- Omit both HTML fields to retain the fallback while adding state or a script.
- `state` or `stateFile` must be a JSON object and is exposed beneath the
  dotted region name, such as `regions.home.afterHero`.
- `scriptFile` is bundled only on pages where the region is active.

Configured names must exist in the active template. State-bearing names cannot
overlap as dotted prefixes, because one JSON value cannot own both `summary` and
`summary.details`.

## Bundled template regions

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
| `page.outline` | `doc`, `page` | Built-in outline when enabled |
| `doc.beforeContent` | `doc` | Empty slot before the article |
| `doc.afterContent` | `doc` | Empty slot after the article |
| `doc.pageNavigation` | `doc` | Previous/next links |
| `doc.footer` | `doc` | Site footer |
| `page.beforeContent` | `page` | Empty slot before wide content |
| `page.afterContent` | `page` | Empty slot after wide content |
| `page.footer` | `page` | Wide page footer |
| `full.beforeContent` | `full` | Empty slot before viewport content |
| `full.afterContent` | `full` | Empty slot after viewport content |

`home.*` applies to the generated home page. A custom page with
`layout: "home"` retains the non-home shell and therefore uses `doc.*` regions.
For complete template replacement, use
`webui press build --template <TEMPLATE_DIR>` or the equivalent `serve`
subcommand.
