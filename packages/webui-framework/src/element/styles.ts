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
 * - **Module**: `<style type="module" specifier="...">` definitions are emitted
 *   in `<head>` during SSR for all inventoried components. The framework reads
 *   these definitions, creates a single `CSSStyleSheet` per specifier (cached in
 *   `sheetCache`), and adopts it onto each shadow root that needs it. During SPA
 *   navigation the router appends new definitions to `<head>` before template
 *   scripts execute.
 *
 * For light DOM components (no shadow root), Module mode falls back to injecting
 * a `<style>` element in `<head>`, deduplicated by `headInjected`.
 */

/**
 * Cached constructable CSSStyleSheet instances keyed by specifier.
 * Parsed once from the `<style type="module">` definition in the document,
 * then adopted onto every shadow root that references the same specifier.
 */
const sheetCache = new Map<string, CSSStyleSheet>();

/**
 * Specifiers already injected into `<head>` via the light DOM path.
 * Prevents duplicate `<style>` elements for non-shadow components.
 */
const headInjected = new Set<string>();

/**
 * Inject a CSS module stylesheet. Shadow DOM components get a
 * constructable stylesheet on their shadow root; light DOM components
 * get a `<style>` in the document head. The CSSStyleSheet is parsed
 * once per specifier and reused across all shadow roots that adopt it.
 */
export function injectModuleStyle(
  specifier: string,
  shadowRoot: ShadowRoot | null,
): void {
  // Fast path: reuse a cached sheet for shadow DOM components
  let sheet = sheetCache.get(specifier);
  if (sheet) {
    if (shadowRoot && !shadowRoot.adoptedStyleSheets.includes(sheet)) {
      shadowRoot.adoptedStyleSheets = [
        ...shadowRoot.adoptedStyleSheets,
        sheet,
      ];
    }
    return;
  }

  // Find the <style type="module" specifier="..."> definition in the document
  const defs = document.querySelectorAll('style[type="module"][specifier]');
  let cssText: string | null = null;
  for (let i = 0; i < defs.length; i++) {
    if (defs[i].getAttribute('specifier') === specifier) {
      cssText = defs[i].textContent;
      break;
    }
  }
  if (!cssText) return;

  if (shadowRoot) {
    sheet = new CSSStyleSheet();
    sheet.replaceSync(cssText);
    sheetCache.set(specifier, sheet);
    shadowRoot.adoptedStyleSheets = [
      ...shadowRoot.adoptedStyleSheets,
      sheet,
    ];
  } else if (!headInjected.has(specifier)) {
    headInjected.add(specifier);
    const style = document.createElement('style');
    style.textContent = cssText;
    document.head.appendChild(style);
  }
}