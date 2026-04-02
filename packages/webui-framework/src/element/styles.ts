// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Stylesheet management for WebUI components.
 *
 * Handles three CSS delivery strategies:
 * - **link** — `<link rel="stylesheet">` per shadow root, optimized via adoptedStyleSheets
 * - **style** — inline `<style>` in shadow root, optimized via adoptedStyleSheets
 * - **module** — constructable stylesheets via `<style type="module">` definitions
 *
 * SSR hydration steals browser-parsed CSSStyleSheet objects from `<link>` elements,
 * caches them, and adopts them — eliminating duplicate style recalculations.
 * Client-created components reuse cached sheets with zero network requests.
 */

// ── Caches ──────────────────────────────────────────────────────

/** Shared constructable stylesheets keyed by href — avoids per-instance
 *  `<link>` elements and duplicate style recalculations in shadow DOM. */
const sheetCache = new Map<string, CSSStyleSheet>();

/** Pre-processed template HTML with `<link>` tags stripped, keyed by
 *  original `meta.h`. Used for client-created shadow DOM components. */
const strippedTemplateCache = new Map<string, { html: string; hrefs: string[] }>();

/** CSS modules already injected into the document head. */
const injectedStyles = new Set<string>();

/**
 * Check if all stylesheet hrefs are already cached from a prior SSR instance.
 * Used to decide whether client-created shadow DOM can adopt cached sheets
 * instead of keeping `<link>` tags.
 */
export function allSheetsCached(hrefs: string[]): boolean {
  return hrefs.length > 0 && hrefs.every(h => sheetCache.has(h));
}

// ── Link tag stripping ──────────────────────────────────────────

/**
 * Strip `<link rel="stylesheet" href="...">` tags from template HTML
 * iteratively (no regex). Returns the cleaned HTML and extracted hrefs.
 */
export function stripLinkTags(html: string): { html: string; hrefs: string[] } {
  const cached = strippedTemplateCache.get(html);
  if (cached) return cached;

  const hrefs: string[] = [];
  const chunks: string[] = [];
  let chunkStart = 0;
  let i = 0;
  while (i < html.length) {
    if (html.charCodeAt(i) === 60 && html.charCodeAt(i + 1) === 108 /* l */
        && html.substring(i, i + 5) === '<link') {
      const tagEnd = html.indexOf('>', i);
      if (tagEnd === -1) { i++; continue; }
      const tag = html.substring(i, tagEnd + 1);
      if (tag.indexOf('rel="stylesheet"') !== -1) {
        const hrefStart = tag.indexOf('href="');
        if (hrefStart !== -1) {
          const hrefValStart = hrefStart + 6;
          const hrefEnd = tag.indexOf('"', hrefValStart);
          if (hrefEnd !== -1) {
            hrefs.push(tag.substring(hrefValStart, hrefEnd));
          }
        }
        if (i > chunkStart) chunks.push(html.substring(chunkStart, i));
        chunkStart = tagEnd + 1;
        i = chunkStart;
        continue;
      }
    }
    i++;
  }
  if (chunkStart < html.length) chunks.push(html.substring(chunkStart));

  const entry = { html: chunks.join(''), hrefs };
  strippedTemplateCache.set(html, entry);
  return entry;
}

// ── Adopted stylesheets ─────────────────────────────────────────

/**
 * Adopt shared stylesheets on a shadow root during SSR hydration.
 * Extracts the already-parsed CSSStyleSheet from each `<link>` element,
 * caches it for reuse by future instances, and removes the `<link>` node.
 */
export function adoptSSRStyles(shadowRoot: ShadowRoot): void {
  const links = shadowRoot.querySelectorAll('link[rel="stylesheet"]');
  if (links.length === 0) return;

  const sheets: CSSStyleSheet[] = [];
  for (let i = 0; i < links.length; i++) {
    const link = links[i] as HTMLLinkElement;
    const href = link.href;

    let cached = sheetCache.get(href);
    if (!cached && link.sheet) {
      cached = new CSSStyleSheet();
      const rules: string[] = [];
      for (let r = 0; r < link.sheet.cssRules.length; r++) {
        rules.push(link.sheet.cssRules[r].cssText);
      }
      try { cached.replaceSync(rules.join('\n')); } catch { /* security */ }
      sheetCache.set(href, cached);
    }
    if (cached) {
      sheets.push(cached);
      link.remove();
    }
  }

  if (sheets.length > 0) {
    const existing = shadowRoot.adoptedStyleSheets;
    if (existing.length === 0) {
      shadowRoot.adoptedStyleSheets = sheets;
    } else {
      shadowRoot.adoptedStyleSheets = existing.concat(sheets);
    }
  }
}

/**
 * Adopt pre-cached stylesheets on a newly created shadow root.
 * Called for client-created components where `<link>` tags have been
 * stripped from the template HTML.
 */
export function adoptCachedStyles(shadowRoot: ShadowRoot, hrefs: string[]): void {
  if (hrefs.length === 0) return;

  const sheets: CSSStyleSheet[] = [];
  for (let i = 0; i < hrefs.length; i++) {
    let cached = sheetCache.get(hrefs[i]);
    if (!cached) {
      cached = new CSSStyleSheet();
      sheetCache.set(hrefs[i], cached);
      fetch(hrefs[i]).then(r => r.text()).then(css => {
        try { cached!.replaceSync(css); } catch { /* ignore */ }
      });
    }
    sheets.push(cached);
  }

  const existing = shadowRoot.adoptedStyleSheets;
  if (existing.length === 0) {
    shadowRoot.adoptedStyleSheets = sheets;
  } else {
    shadowRoot.adoptedStyleSheets = existing.concat(sheets);
  }
}

// ── CSS module injection ────────────────────────────────────────

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
