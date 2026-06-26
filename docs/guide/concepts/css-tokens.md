# CSS Token Hoisting

CSS Token Hoisting is a build-time optimization that discovers which CSS custom properties (design tokens) your application actually uses, and includes only those in the protocol output. This enables host runtimes to resolve design tokens efficiently without shipping unused variables.

## How It Works

During the build process, WebUI extracts CSS custom property **usages** - names referenced via `var(--name)` - from two sources:

1. **Component CSS files** - tokens are extracted when components are registered and cached in the component registry.
2. **Inline `<style>` tags** - tokens are extracted from `<style>` elements in the entry HTML file and component templates.

The resulting set of tokens is sorted, deduplicated, and included in the protocol's `tokens` field.

### What Gets Hoisted

Only `var()` **usages** are hoisted - not custom property **definitions**:

```css
/* ✅ HOISTED - usage via var() */
.button {
  color: var(--colorPrimary);           /* → "colorPrimary" */
  background: var(--bgColor);           /* → "bgColor" */
}

/* ❌ NOT hoisted - this is a definition */
:root {
  --colorPrimary: #0078d4;
}
```

### Nested Fallbacks

Fallback variables in `var()` calls are also extracted:

```css
.card {
  /* All three tokens are extracted: "a", "b", "c" */
  color: var(--a, var(--b, var(--c)));
}
```

Literal fallback values (like `16px`) are ignored - only variable references are extracted.
Malformed `var()` calls fail the build instead of hoisting partial token names.

### Local Definition Exclusion

If a custom property is defined in the same CSS file, or by an ancestor
component/root CSS scope, matching token candidates are **excluded** from the
token set. This prevents locally-scoped variables from being hoisted:

```css
:host {
  --internal-spacing: 8px;             /* definition */
  padding: var(--internal-spacing);     /* usage */
  color: var(--designSystemColor);      /* usage only → HOISTED */
}
/* Result: only "designSystemColor" is hoisted */
```

Definitions are excluded even when the variable appears in a nested fallback:

```css
:host {
  --token-a: red;
  --foo-bar: var(--token-a, var(--token-b, var(--token-c)));
}
/* Result: "token-b" and "token-c" are hoisted; "token-a" is local */
```

### Theme Validation

When a build is given a theme (`webui build --theme`, `webui serve --theme`, or
API build options with a theme), every **required** token must exist in every
theme. For `var(--a, var(--b, var(--c)))`, the theme must provide `a`, `b`, and
`c` unless any of those tokens are defined by local or ancestor CSS. Missing
tokens fail with `missing-theme-token`; theme token values that reference an
undefined or cyclic `var(--token)` are trusted and left to browser CSS
semantics.

Both the error and the typo advisory point at the offending CSS
(`--> my-card.css:2:10`, with the source line) and suggest the closest theme
token by edit distance, so a misspelled `var(--color-neutral-2000)` reports:

```
✘ error: missing theme token [missing-theme-token]
  --> my-card.css:2:10
    color: var(--color-neutral-2000);
  help: did you mean --color-neutral-200? otherwise define it locally
```


A `var()` usage that supplies a **literal CSS fallback** is exempt — the literal
already provides a value, so the token is not required:

```css
:host {
  color: var(--brand, #000);   /* not required: #000 is the fallback */
  margin: var(--gap);          /* required: no fallback */
}
```

`--brand` is still hoisted into the protocol so the runtime resolves it when a
theme *does* define it, but its absence does not fail the build. If the same
token is also used without a fallback anywhere (e.g. a bare `var(--brand)`), it
becomes required again.

As a safety net for typos, a token used **only** with a literal fallback and
defined in **no** theme (e.g. a misspelled `var(--colr-brand, #000)`) is
reported as a non-fatal advisory — `webui build` prints it and it is available
on `BuildResult::warnings` — rather than failing the build.

## Comment Handling

HTML and CSS comments are stripped at build time. Bindings inside HTML comments
are ignored, so comments cannot be used as token placeholders. CSS legal
comments are preserved only when `--legal-comments inline` is active.
Unterminated HTML or CSS comments fail the build.

Inside `<style>` tags, dynamic CSS fragments are valid only when wrapped in a
CSS block comment. The comment must contain exactly one handlebars expression:

```html
<style>
  :root {
    /*{{{tokens.light}}}*/
  }
</style>
```

## Protocol Output

The hoisted tokens appear in the protocol's `tokens` field:

```json
{
  "fragments": { ... },
  "tokens": [
    "bgColor",
    "borderRadiusSmall",
    "colorBrandBackground",
    "colorPrimary",
    "fontFamilyBase",
    "lineHeightBase400",
    "spacingHorizontalM"
  ]
}
```

The list is always **sorted alphabetically** and **deduplicated** across all components and inline styles.

## CLI Output

When tokens are discovered, the build command reports the count:

```
✔ Registered 5 components
✔ Parsed index.html (23 fragments)
✔ Discovered 12 CSS tokens
✔ Build complete (3 files written) in 42ms
```
