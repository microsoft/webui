// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { TemplateBlockMeta } from './template-types.js';

// lib.dom does not yet describe Trusted Types. These local opaque types preserve
// the browser objects at the sink; they are never coerced back to strings.
interface TrustedValue {
  toString(): string;
}

interface BrowserPolicy {
  createHTML(input: string, capability: object): TrustedValue;
  createScript(input: string, capability: object): TrustedValue;
}

interface BrowserTrustedTypes {
  createPolicy(name: string, rules: {
    createHTML(input: string, capability: object): string;
    createScript(input: string, capability: object): string;
  }): BrowserPolicy;
}

declare global {
  interface WebUITrustedTemplates {
    readonly policyName: string;
    registerBlock(meta: { h: string }): void;
    setTemplateContent(target: HTMLTemplateElement, meta: { h: string }): void;
    setImportMap(script: HTMLScriptElement, specifier: string, css: string): void;
    installTemplateFunctions(functions: Record<string, string>, nonce: string): void;
  }

  interface Window {
    __webuiTrustedTemplates?: WebUITrustedTemplates;
  }
}

/**
 * Opt in to Trusted Types for WebUI compiler output in this document.
 *
 * Call before importing/defining components or starting the router/streaming
 * runtime. Allow this exact, non-default policy name in CSP's `trusted-types`
 * directive. Repeating the same name is idempotent across entry bundles.
 *
 * This is not a sanitizer: only trusted compiler metadata, condition closures,
 * and generated CSS import maps enter this policy. State/raw HTML, FAST string
 * templates, script URLs and arbitrary application scripts are not authorized.
 * Browsers without Trusted Types retain their existing behavior.
 */
export function configureTrustedTypes(policyName: string): void {
  validatePolicyName(policyName);
  const existing = window.__webuiTrustedTemplates;
  if (existing) {
    if (existing.policyName !== policyName) {
      throw new Error(`[WebUI] Trusted Types already configured as "${existing.policyName}"; configure one name before loading the application.`);
    }
    return;
  }
  const factory = (window as Window & { trustedTypes?: BrowserTrustedTypes }).trustedTypes;
  if (!factory) return;
  const capability = {};
  const authorize = (input: string, token: object): string => {
    if (token !== capability) {
      throw new TypeError('[WebUI] Only the compiled-template runtime may use this Trusted Types policy.');
    }
    return input;
  };
  let policy: BrowserPolicy;
  try {
    policy = factory.createPolicy(policyName, {
      createHTML: authorize,
      createScript: authorize,
    });
  } catch (cause) {
    throw new Error(`[WebUI] Cannot create Trusted Types policy "${policyName}". Allow that exact name in CSP and let configureTrustedTypes() create it before loading the application.`, { cause });
  }
  const blocks = new WeakMap<object, string>();
  const boundary: WebUITrustedTemplates = {
    policyName,
    registerBlock(meta) {
      if (!blocks.has(meta)) blocks.set(meta, meta.h);
    },
    setTemplateContent(target, meta) {
      if (!blocks.has(meta) || blocks.get(meta) !== meta.h) {
        throw new Error('[WebUI] Unregistered or modified compiled template. Configure Trusted Types before importing components; register only immutable compiler output.');
      }
      // A type assertion only bridges the incomplete DOM declaration. The
      // runtime value remains TrustedHTML, and the native setter enforces it.
      target.innerHTML = policy.createHTML(meta.h, capability) as string & TrustedValue;
    },
    setImportMap(script, specifier, css) {
      const json = JSON.stringify({
        imports: { [specifier]: `data:text/css,${encodeURIComponent(css)}` },
      });
      script.textContent = policy.createScript(json, capability) as string & TrustedValue;
    },
    installTemplateFunctions(functions, nonce) {
      const tags = Object.keys(functions);
      if (tags.length === 0) return;
      let body = '(function(){var w=(window.__webui||(window.__webui={}));var f=w.templateFns||(w.templateFns={});';
      for (let i = 0; i < tags.length; i++) {
        const tag = tags[i];
        const source = functions[tag];
        if (!source) continue;
        body += `f[${JSON.stringify(tag)}]=${source};`;
      }
      body += '})();';
      const script = document.createElement('script');
      if (nonce) script.nonce = nonce;
      script.textContent = policy.createScript(body, capability) as string & TrustedValue;
      document.head.appendChild(script);
      script.remove();
    },
  };
  // One boundary per document also serves split/duplicated module entrypoints.
  Object.defineProperty(window, '__webuiTrustedTemplates', { value: Object.freeze(boundary) });
}

function validatePolicyName(name: string): void {
  if (typeof name !== 'string' || name.length === 0 || name === 'default') {
    throw new TypeError('[WebUI] Supply an explicit non-default Trusted Types policy name.');
  }
  for (let i = 0; i < name.length; i++) {
    const c = name.charCodeAt(i);
    if (
      (c >= 65 && c <= 90) || (c >= 97 && c <= 122) ||
      (c >= 48 && c <= 57) || c === 45 || c === 46 || c === 95
    ) continue;
    throw new TypeError('[WebUI] Trusted Types policy names must contain only ASCII letters, digits, ".", "_" or "-".');
  }
}

/** Record immutable compiler HTML during template normalization, never state updates. */
export function registerTrustedTemplateBlock(meta: TemplateBlockMeta): void {
  window.__webuiTrustedTemplates?.registerBlock(meta);
}
