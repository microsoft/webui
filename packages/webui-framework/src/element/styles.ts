// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Stylesheet management for WebUI components.
 *
 * Three CSS strategies are supported:
 *
 * - **Link**: `<link rel="stylesheet">` tags in each component's shadow template.
 *   The browser deduplicates fetches by URL. No JS-side style management needed.
 *
 * - **Style**: Inline `<style>` tags inside each shadow template.
 *
 * - **Module**: Uses the Declarative CSS Module Scripts proposal. During SSR,
 *   `<style type="module" specifier="...">` definitions are emitted inline in
 *   each rendered component's light DOM. The browser registers these globally
 *   and automatically adopts them via `shadowrootadoptedstylesheets` on
 *   declarative shadow roots.
 *
 *   During SPA navigation, the router appends new `<style type="module">`
 *   definitions to `<head>` via `templateStyles[]`. The framework uses
 *   `import(specifier, { with: { type: "css" } })` to retrieve the browser's
 *   registered CSSStyleSheet and adopts it onto the shadow root. This is a
 *   direct hash-map lookup in the browser's module registry — no DOM queries,
 *   no manual CSSStyleSheet construction.
 *
 * For light DOM components (no shadow root), Module mode injects a `<style>`
 * element in `<head>`, deduplicated by `headInjected`.
 */

/**
 * Specifiers already injected into `<head>` via the light DOM path.
 * Prevents duplicate `<style>` elements for non-shadow components.
 */
const headInjected = new Set<string>();

/**
 * Resolved CSSStyleSheet cache for the shadow-DOM SPA path.
 *
 * Each unique component shipped with a CSS module produces one
 * `import(specifier, { with: { type: 'css' } })` call. Without caching, every
 * shadow instance of a repeated component (e.g. 150 email rows on an inbox)
 * allocates its own Promise + `.then` continuation. By memoising the resolved
 * sheet here, subsequent instances adopt synchronously — eliminating the
 * Promise + closure + adoptedStyleSheets-spread allocations on lists.
 *
 * Values:
 *   - `Promise<CSSStyleSheet>` — import in flight (concurrent instances await)
 *   - `CSSStyleSheet`          — resolved (synchronous adoption)
 *   - `null`                   — import failed; subsequent calls become no-ops
 */
const moduleSheetCache = new Map<string, CSSStyleSheet | Promise<CSSStyleSheet> | null>();

function adoptSheet(shadowRoot: ShadowRoot, sheet: CSSStyleSheet): void {
  // adoptedStyleSheets is an ObservableArray (FrozenArray-like in legacy
  // browsers, mutable in modern ones).  Modern Chrome, Edge and Firefox
  // support `.push()` directly; falling back to assignment uses a single
  // [sheet] allocation when empty (no spread needed).
  const sheets = shadowRoot.adoptedStyleSheets;
  if (sheets.indexOf(sheet) !== -1) return;
  if (typeof (sheets as { push?: (s: CSSStyleSheet) => number }).push === 'function') {
    (sheets as unknown as CSSStyleSheet[]).push(sheet);
    return;
  }
  shadowRoot.adoptedStyleSheets = sheets.length === 0
    ? [sheet]
    : [...sheets, sheet];
}

/**
 * Adopt a CSS module stylesheet onto a shadow root, or inject into `<head>`
 * for light DOM components.
 *
 * For shadow DOM: uses `import(specifier, { with: { type: "css" } })` to
 * retrieve the browser-registered CSSStyleSheet from the module registry.
 * The browser caches the sheet internally — no application-level cache needed.
 *
 * For light DOM: appends a `<style>` element to `<head>` (once per specifier).
 */
export function injectModuleStyle(
  specifier: string,
  shadowRoot: ShadowRoot | null,
): void {
  if (shadowRoot) {
    // SSR hydration: the browser already adopted the sheet from
    // shadowrootadoptedstylesheets on the declarative shadow root.
    if (shadowRoot.adoptedStyleSheets.length > 0) return;

    // SPA path: import the CSS module from the browser's registry.
    // Memoise the resolved sheet by specifier so repeated component instances
    // adopt synchronously instead of allocating a fresh Promise per instance.
    const cached = moduleSheetCache.get(specifier);
    if (cached === null) return;
    if (cached instanceof CSSStyleSheet) {
      adoptSheet(shadowRoot, cached);
      return;
    }
    const promise = cached ?? import(specifier, { with: { type: 'css' } }).then(
      (mod: { default: CSSStyleSheet }) => {
        moduleSheetCache.set(specifier, mod.default);
        return mod.default;
      },
      () => {
        moduleSheetCache.set(specifier, null);
        return null as unknown as CSSStyleSheet;
      },
    );
    if (cached === undefined) moduleSheetCache.set(specifier, promise);
    promise.then((sheet) => {
      if (sheet) adoptSheet(shadowRoot, sheet);
    });
  } else if (!headInjected.has(specifier)) {
    headInjected.add(specifier);
    import(specifier, { with: { type: 'css' } }).then(
      (mod: { default: CSSStyleSheet }) => {
        const style = document.createElement('style');
        const rules = mod.default.cssRules;
        let cssText = '';
        for (let i = 0; i < rules.length; i++) {
          cssText += rules[i].cssText;
        }
        style.textContent = cssText;
        document.head.appendChild(style);
      },
      () => {},
    );
  }
}