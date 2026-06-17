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
    templates?: Record<string, unknown>;
    templateFunctions?: Record<string, string>;
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

  // 1. CSS modules: each entry is a `<script type="importmap">` tag
  //    registering one or more component CSS modules under a data:text/css
  //    URI. Append one importmap script per entry (matching the SSR
  //    handler's 1:1 emission) so SSR and SPA produce the same DOM shape.
  //    Multiple Import Maps support (required for this strategy) handles
  //    the document-level merge.
  if (data.templateStyles) {
    for (const scriptMarkup of data.templateStyles) {
      const trimmed = scriptMarkup.trim();
      if (!trimmed.startsWith('<script')) continue;

      const openTagEnd = trimmed.indexOf('>');
      const closeTagStart = trimmed.lastIndexOf('</script>');
      if (openTagEnd < 0 || closeTagStart <= openTagEnd) continue;

      const jsonBody = trimmed.substring(openTagEnd + 1, closeTagStart);
      let parsed: { imports?: Record<string, unknown> };
      try {
        parsed = JSON.parse(jsonBody) as { imports?: Record<string, unknown> };
      } catch {
        continue;
      }
      if (!parsed.imports || typeof parsed.imports !== 'object') continue;

      const newImports: Record<string, string> = {};
      let hasNew = false;
      for (const [specifier, uri] of Object.entries(parsed.imports)) {
        if (typeof uri !== 'string' || !uri.startsWith('data:text/css,')) continue;
        if (injectedStyles.has(specifier)) continue;
        newImports[specifier] = uri;
        injectedStyles.add(specifier);
        hasNew = true;
      }
      if (!hasNew) continue;

      const script = document.createElement('script');
      script.type = 'importmap';
      if (nonce) script.nonce = nonce;
      script.textContent = JSON.stringify({ imports: newImports });
      document.head.appendChild(script);
    }
  }

  let executableTemplateBody = '';

  // 2. Template closures: execute only the component-local condition arrays.
  //    TRUST BOUNDARY: closure scripts come from the same-origin server
  //    that compiled the protocol. The CSP nonce gates script execution.
  //    If the server endpoint is compromised, this is an XSS vector —
  //    same risk as the existing fetchPartial pipeline.
  if (data.templateFunctions) {
    const tags = Object.keys(data.templateFunctions);
    if (tags.length > 0) {
      executableTemplateBody += 'var w=(window.__webui||(window.__webui={}));var f=w.templateFns||(w.templateFns={});';
    }
    for (let i = 0; i < tags.length; i++) {
      const tag = tags[i];
      const functions = data.templateFunctions[tag];
      if (!functions) continue;
      executableTemplateBody += 'f[';
      executableTemplateBody += JSON.stringify(tag);
      executableTemplateBody += ']=';
      executableTemplateBody += functions;
      executableTemplateBody += ';';
    }
  }

  // 3. Template metadata: register JSON-safe data directly. Non-WebUI
  //    string payloads (FAST f-template HTML or legacy executable payloads)
  //    keep their materialization path.
  if (data.templates) {
    const w = window as unknown as { __webui?: { templates?: Record<string, unknown>; [key: string]: unknown } };
    if (!w.__webui) w.__webui = {};
    if (!w.__webui.templates) w.__webui.templates = {};
    const tags = Object.keys(data.templates);
    for (let i = 0; i < tags.length; i++) {
      const tag = tags[i];
      const template = data.templates[tag];
      if (typeof template === 'string') {
        if (template.startsWith('<')) {
          const container = document.createDocumentFragment();
          const temp = document.createElement('div');
          temp.innerHTML = template;
          while (temp.firstChild) container.appendChild(temp.firstChild);
          document.body.appendChild(container);
        } else {
          executableTemplateBody += template;
          executableTemplateBody += '\n';
        }
      } else {
        w.__webui.templates[tag] = template;
      }
    }
  }

  if (executableTemplateBody) {
    const script = document.createElement('script');
    if (nonce) script.nonce = nonce;
    script.textContent = `(function(){${executableTemplateBody}})();`;
    document.head.appendChild(script);
    document.head.removeChild(script);
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
