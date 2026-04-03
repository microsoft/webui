// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Stylesheet management for WebUI components.
 *
 * CSS delivery relies on the browser's native `<link>` caching — the handler
 * emits `<link rel="stylesheet">` tags in each component's template, and the
 * browser deduplicates fetches by URL. No adoptedStyleSheets manipulation needed.
 *
 * The only active helper is `injectModuleStyle` for CSS module mode, which
 * activates `<style type="module">` definitions emitted by the handler.
 */

/** CSS modules already injected into the document head. */
const injectedStyles = new Set<string>();

/**
 * Inject a CSS module stylesheet. Shadow DOM components get a
 * constructable stylesheet on their shadow root; light DOM components
 * get a `<style>` in the document head. Each specifier is processed
 * once per page.
 */
export function injectModuleStyle(
  specifier: string,
  shadowRoot: ShadowRoot | null,
): void {
  if (injectedStyles.has(specifier)) return;
  injectedStyles.add(specifier);

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
    const sheet = new CSSStyleSheet();
    sheet.replaceSync(cssText);
    shadowRoot.adoptedStyleSheets = [
      ...shadowRoot.adoptedStyleSheets,
      sheet,
    ];
  } else {
    const style = document.createElement('style');
    style.textContent = cssText;
    document.head.appendChild(style);
  }
}