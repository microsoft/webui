---
title: "CSS tokens and themes"
description: "Use CSS custom properties, load a theme file or package, and validate required tokens without changing CSS ownership."
---

<a id="css-token-hoisting"></a>

# CSS Tokens and Themes

Style components with ordinary CSS custom properties. You can define them in
your own stylesheets, or use `--theme` to validate and supply a shared token
set. Theming does not scope selectors: Light DOM CSS is global in its owning
CSS tree; Shadow DOM provides native isolation.

```css
/* my-card.css, using the default Shadow DOM mode */
:host {
  color: var(--text-primary);
  background: var(--surface-page);
}
```

## Configure a theme

Theme files map token names **without** `--` to CSS value strings:

```json
{
  "themes": {
    "light": {
      "text-primary": "#111827",
      "surface-page": "#ffffff"
    },
    "dark": {
      "text-primary": "#fafafa",
      "surface-page": "#171717"
    }
  }
}
```

A flat object such as `{ "text-primary": "#111827" }` defines one theme named
`default`. Load a local JSON file or an installed npm package:

```bash
webui build ./src --out ./dist --theme ./themes/brand.json
webui serve ./src --state ./data/state.json --theme ./themes/brand.json --watch
```

For a package, `--theme @my-org/brand-tokens` resolves its `tokens.json`;
`@my-org/brand-tokens/custom.json` selects another file.

## Apply theme values

`webui build --theme` validates token coverage; it does not choose the active
theme or add a theme switcher. `webui serve --theme` also puts resolved CSS
declarations in state as `tokens.<theme-name>`. Apply them in the entry
template's `<head>`:

```html
<style>
  :root {
    color-scheme: light dark;
    /*{{{tokens.light}}}*/
  }
  @media (prefers-color-scheme: dark) {
    :root {
      /*{{{tokens.dark}}}*/
    }
  }
</style>
```

Use `tokens.default` for a flat theme file. The CSS-comment wrapper around each
binding is required. These are raw CSS declarations: load trusted theme files,
not user-supplied strings.

In a custom host, resolve tokens and supply the resulting declarations in
render state, or publish them as a stylesheet. The Rust
`microsoft-webui-tokens` crate exposes `load_token_file`, `resolve_tokens`,
and `inject_token_css`; other hosts can read the protocol's token inventory
through their [integration API](/build/integrations/).

<a id="how-it-works"></a>
<a id="what-gets-hoisted"></a>

## Required tokens and defaults

With `--theme`, WebUI checks the `var(--name)` references in your component
CSS and inline `<style>` blocks. Provide those tokens in each theme, define
local defaults, or give the usage a literal fallback.

### Nested Fallbacks

Without local defaults, all three names in this example need theme values:

```css
.card {
  color: var(--a, var(--b, var(--c)));
}
```

Use a literal fallback such as `var(--gap, 16px)` when the token is optional.
Malformed `var()` calls fail the build.

### Local Definition Exclusion

A definition in the same declaration block provides a local default.
Unconditional `:host` or `:root` defaults also apply to descendant components:

```css
:host {
  --card-spacing: 8px;
  padding: var(--card-spacing);
  color: var(--designSystemColor);
}
```

Here, the theme needs `designSystemColor`, but not `card-spacing`.

Selector-scoped overrides do not replace theme defaults outside their matching
elements. For example, `.green { --brand: var(--green-brand); }` still requires
the theme's `brand` token when another rule or child component uses
`var(--brand)`. Conditional defaults inside `@media`, `@supports`, or
`@container` likewise do not replace unconditional defaults. Put shared
Shadow component defaults in a bare `:host` rule and document defaults in
`:root`; these can also live inside cascade layers. `:host` is not valid in
Light component CSS.

### Theme Validation

When a build is given a theme (`webui build --theme`, `webui serve --theme`, or
API build options with a theme), every **required** token must exist in every
theme. For `var(--a, var(--b, var(--c)))`, the theme must provide `a`, `b`, and
`c` unless those references are covered by same-block definitions or
unconditional local/ancestor defaults. Missing
tokens fail with `missing-theme-token`; theme token values that reference an
undefined or cyclic `var(--token)` are trusted and left to browser CSS
semantics.

Errors point at the CSS that needs attention and suggest a matching theme token:

```
✘ error: missing theme token [missing-theme-token]
  --> my-card.css:2:10
    color: var(--color-neutral-2000);
  help: did you mean --color-neutral-200? otherwise define it locally
```


A `var()` usage that supplies a **literal CSS fallback** is exempt: the literal
already provides a value, so the token is not required:

```css
:host {
  color: var(--brand, #000);   /* not required: #000 is the fallback */
  margin: var(--gap);          /* required: no fallback */
}
```

The theme can still supply `brand`; the literal is used when no value is
available. If another usage has no fallback, `brand` becomes required.

A token used only with a literal fallback but missing from every theme produces
a non-fatal typo advisory. `webui build` prints it; library callers can inspect
`BuildResult::warnings`.

## Comment Handling

HTML and CSS comments are stripped at build time. Bindings inside HTML comments
are ignored, so comments cannot be used as token placeholders. CSS legal
comments are preserved only when `--legal-comments inline` is active.
Unterminated HTML or CSS comments fail the build.

Use the CSS-comment-wrapped theme bindings shown under
[Apply theme values](#apply-theme-values). The comment must contain exactly
one binding.

<a id="protocol-output"></a>
<a id="cli-output"></a>

For inspecting a compiled build, see the [CLI reference](/reference/cli/).
