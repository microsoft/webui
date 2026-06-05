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

### Local Definition Exclusion

If a custom property is both **defined** and **used** in the same CSS file, it is **excluded** from the token set. This prevents locally-scoped variables from being hoisted:

```css
:host {
  --internal-spacing: 8px;             /* definition */
  padding: var(--internal-spacing);     /* usage */
  color: var(--designSystemColor);      /* usage only → HOISTED */
}
/* Result: only "designSystemColor" is hoisted */
```

## Comment Handling

HTML and CSS comments are stripped at build time. Bindings inside HTML comments
are ignored, so comments cannot be used as token placeholders. CSS legal
comments are preserved only when `--legal-comments inline` is active.

Inject resolved token declarations through normal state bindings instead:

```html
<style>
  :root {
    {{{tokens.light}}}
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
