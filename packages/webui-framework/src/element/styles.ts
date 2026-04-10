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
    // The <style type="module" specifier="X"> definition was either
    // emitted inline during SSR or appended to <head> by the router.
    // The import resolves to the same CSSStyleSheet the browser registered.
    import(specifier, { with: { type: 'css' } }).then(
      (mod: { default: CSSStyleSheet }) => {
        shadowRoot.adoptedStyleSheets = [
          ...shadowRoot.adoptedStyleSheets,
          mod.default,
        ];
      },
      () => {
        // Specifier not registered — component has no CSS module definition.
        // This is expected for Link/Style strategies or components without CSS.
      },
    );
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