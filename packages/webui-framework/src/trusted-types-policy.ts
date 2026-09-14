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

interface CompilerPolicy {
  readonly policyName: string;
  registerBlock(meta: TemplateBlockMeta): void;
  setTemplateContent(target: HTMLTemplateElement, meta: TemplateBlockMeta): void;
  setImportMap(script: HTMLScriptElement, json: string): void;
}

let documentPolicies: WeakMap<Window, CompilerPolicy> | undefined;

declare global {
  interface Window {
    readonly __webuiTrustedTypesPolicyName?: string;
  }
}

/**
 * Opt in to Trusted Types for WebUI compiler output in this document.
 *
 * Call before importing/defining components or starting an optional hydration
 * runtime. Allow this exact, non-default policy name in CSP's `trusted-types`
 * directive. Repeating the same name is idempotent when entry bundles share one
 * framework module instance; independently bundled copies are rejected.
 *
 * This is not a sanitizer: only trusted compiler metadata and generated CSS
 * import maps enter this policy. State/raw HTML, FAST string
 * templates, script URLs and arbitrary application scripts are not authorized.
 * Browsers without Trusted Types retain their existing behavior.
 */
export function configureTrustedTypes(policyName: string): void {
  validatePolicyName(policyName);
  const existing = getDocumentPolicy(window);
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
  const boundary: CompilerPolicy = {
    policyName,
    registerBlock(meta) {
      if (!blocks.has(meta)) blocks.set(meta, meta.h);
    },
    setTemplateContent(target, meta) {
      const html = meta.h;
      if (!blocks.has(meta) || blocks.get(meta) !== html) {
        throw new Error('[WebUI] Unregistered or modified compiled template. Configure Trusted Types before importing components; register only immutable compiler output.');
      }
      // A type assertion only bridges the incomplete DOM declaration. The
      // runtime value remains TrustedHTML, and the native setter enforces it.
      target.innerHTML = policy.createHTML(html, capability) as string & TrustedValue;
    },
    setImportMap(script, json) {
      script.textContent = policy.createScript(json, capability) as string & TrustedValue;
    },
  };
  // Only inert idempotency metadata crosses module boundaries, never a policy
  // or a callable closure that could supply its private capability.
  Object.defineProperty(window, '__webuiTrustedTypesPolicyName', { value: policyName });
  (documentPolicies ??= new WeakMap()).set(window, boundary);
}

function getDocumentPolicy(view: Window | null): CompilerPolicy | undefined {
  if (!view) return undefined;
  const policy = documentPolicies?.get(view);
  if (!policy && view.__webuiTrustedTypesPolicyName !== undefined) {
    throw new Error('[WebUI] Trusted Types was configured by another framework module instance. Share one framework module across bootstrap and component bundles instead of bundling independent copies.');
  }
  return policy;
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
  getDocumentPolicy(window)?.registerBlock(meta);
}

/** @internal Parse registered compiler HTML without exposing a trusted value. */
export function setTemplateContent(target: HTMLTemplateElement, meta: TemplateBlockMeta): void {
  const policy = getDocumentPolicy(window);
  if (policy) policy.setTemplateContent(target, meta);
  else target.innerHTML = meta.h;
}

/** @internal Serialize CSS data into an inert import map, never executable source. */
export function setImportMapContent(
  script: HTMLScriptElement,
  specifier: string,
  css: string,
  view: Window | null,
): void {
  if (script.type !== 'importmap') {
    throw new TypeError('[WebUI] CSS import map content requires a script with type="importmap".');
  }
  const json = JSON.stringify({
    imports: { [specifier]: `data:text/css,${encodeURIComponent(css)}` },
  });
  const policy = getDocumentPolicy(view);
  if (policy) policy.setImportMap(script, json);
  else script.textContent = json;
}
