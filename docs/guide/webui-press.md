# WebUI Press named regions

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
`webui-press build --template <TEMPLATE_DIR>` or the equivalent `serve`
subcommand.
