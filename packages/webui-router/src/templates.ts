// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Template & CSS registration — shared by partial navigation and
 * `ensureLoaded()`.
 */

/**
 * Register templates + inject CSS from a server response.
 * Shared by fetchPartial and fetchComponentTemplates.
 */
export function registerTemplatesAndStyles(
  data: {
    templates?: string[];
    templateStyles?: string[];
    inventory?: string;
  },
  nonce: string,
  injectedStyles: Set<string>,
  updateInventory: (inv: string) => void,
): void {
  if (data.inventory) {
    updateInventory(data.inventory);
  }

  // 1. Module CSS: inject <style type="module"> definitions into <head>
  if (data.templateStyles) {
    for (const styleMarkup of data.templateStyles) {
      const trimmed = styleMarkup.trim();
      if (!trimmed.startsWith('<style')) continue;

      const openTagEnd = trimmed.indexOf('>');
      const closeTagStart = trimmed.lastIndexOf('</style>');
      if (openTagEnd < 0 || closeTagStart <= openTagEnd) continue;

      const specifierToken = 'specifier="';
      const specStart = trimmed.indexOf(specifierToken);
      let specifier: string | null = null;
      if (specStart >= 0) {
        const valStart = specStart + specifierToken.length;
        const valEnd = trimmed.indexOf('"', valStart);
        if (valEnd > valStart) specifier = trimmed.substring(valStart, valEnd);
      }

      if (specifier && injectedStyles.has(specifier)) {
        continue;
      }

      const style = document.createElement('style');
      style.type = 'module';
      if (specifier) {
        style.setAttribute('specifier', specifier);
        injectedStyles.add(specifier);
      }
      style.textContent = trimmed.substring(openTagEnd + 1, closeTagStart);
      document.head.appendChild(style);
    }
  }

  // 2. Template registration: execute JS IIFEs / insert DOM templates.
  //    TRUST BOUNDARY: template scripts come from the same-origin server
  //    that compiled the protocol. The CSP nonce gates script execution.
  //    If the server endpoint is compromised, this is an XSS vector —
  //    same risk as the existing fetchPartial pipeline.
  if (data.templates) {
    let scriptBody = '';
    for (const tmpl of data.templates) {
      if (tmpl.startsWith('<')) {
        const container = document.createDocumentFragment();
        const temp = document.createElement('div');
        temp.innerHTML = tmpl;
        while (temp.firstChild) container.appendChild(temp.firstChild);
        document.body.appendChild(container);
      } else {
        if (scriptBody) scriptBody += '\n';
        scriptBody += tmpl;
      }
    }
    if (scriptBody) {
      const script = document.createElement('script');
      if (nonce) script.nonce = nonce;
      script.textContent = scriptBody;
      document.head.appendChild(script);
      document.head.removeChild(script);
    }
  }
}

/** Inject CSS stylesheet links from a partial response. */
export function injectCssLinks(
  data: { css?: string[] },
  injectedCss: Set<string>,
): void {
  if (data.css) {
    for (const href of data.css) {
      if (!injectedCss.has(href)) {
        injectedCss.add(href);
        const link = document.createElement('link');
        link.rel = 'stylesheet';
        link.href = href;
        document.head.appendChild(link);
      }
    }
  }
}

/**
 * Fetch component templates + CSS from the server and register them.
 * Reuses the same registration logic as fetchPartial.
 * Throws on network or server errors so callers can handle failures.
 */
export async function fetchComponentTemplates(
  tags: string[],
  inventoryHex: string,
  templateEndpoint: string,
  nonce: string,
  injectedStyles: Set<string>,
  updateInventory: (inv: string) => void,
): Promise<void> {
  const url = `${templateEndpoint}?t=${tags.join(',')}&inv=${encodeURIComponent(inventoryHex)}`;
  const resp = await fetch(url);
  if (!resp.ok) {
    throw new Error(`[Router] ensureLoaded failed: ${resp.status} ${resp.statusText}`);
  }
  const data = await resp.json();

  // Register using the same pipeline as partial navigation
  registerTemplatesAndStyles(data, nonce, injectedStyles, updateInventory);
}
