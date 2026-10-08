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
Native npm discovery names components from `<tag>.html` files across the package
tree. Package `webui.components` metadata can restrict ordinary HTML discovery
to explicit directories, for example `["./src/components"]`. Folder names,
template exports, and CEM names do not determine native component names. Use
package module exports for browser registrations. See
[External components](/guide/concepts/components#external-component-sources).

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

`state` must be an inline JSON-compatible object. Nested objects, arrays,
strings, finite numbers, booleans, and `null` values are supported. Object
keys must be strings; quote numeric or boolean keys. Omit `state` or use `{}`
when no page-local data is needed. A top-level `null`, scalar, array, custom YAML tag,
non-finite number, or malformed YAML fails the build with the Markdown filename
and a corrective hint. Markdown frontmatter does not support `stateFile`.

Global config `state` or `stateFile` supplies shared defaults. Markdown state
then **shallowly replaces matching top-level keys** for that page only. In the
example above, local `example` replaces the entire global `example` object;
its nested fields are not deep-merged. Other global keys remain available,
and local data never flows into other Markdown pages or the generated 404.

Press owns these top-level keys: `site`, `navigation`, `sidebar`, `page`,
`hero`, `footer`, `prev`, `next`, `pageData`, `regions`, `headTags`, `tokens`,
`label`, and `icon`. Using any of them directly under Markdown `state` fails
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

`layout` is optional. Without it, the region is active on every layout.

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
