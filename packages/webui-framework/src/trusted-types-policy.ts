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
  if (getDocumentPolicy(window)) {
    const existing = window.__webuiTrustedTypesPolicyName;
    if (existing !== policyName) {
      throw new Error(`[WebUI] Trusted Types already configured as "${existing}". Use that policy name.`);
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
    throw new Error(`[WebUI] Cannot create policy "${policyName}". Allow that exact name in CSP; let WebUI create it before loading components.`, { cause });
  }
  const blocks = new WeakMap<TemplateBlockMeta, string>();
  // Keep opt-in sink checks inside configuration so unused policy code can be
  // tree-shaken out of applications that never enable Trusted Types.
  const trust: CompilerPolicy = {
    registerBlock(meta) {
      if (!blocks.has(meta)) blocks.set(meta, meta.h);
    },
    setTemplateContent(target, meta) {
      const html = blocks.get(meta);
      if (html === undefined || html !== meta.h) {
        throw new Error('[WebUI] Unregistered or modified compiled template. Configure before loading components; register only immutable compiler output.');
      }
      target.innerHTML = policy.createHTML(html, capability) as string & TrustedValue;
    },
    setImportMap(script, json) {
      script.textContent = policy.createScript(json, capability) as string & TrustedValue;
    },
  };
  // Only inert idempotency metadata crosses module boundaries, never a policy
  // or a callable closure that could supply its private capability.
  Object.defineProperty(window, '__webuiTrustedTypesPolicyName', { value: policyName });
  (documentPolicies ??= new WeakMap()).set(window, trust);
}

function getDocumentPolicy(view: Window | null): CompilerPolicy | undefined {
  if (!view) return undefined;
  const policy = documentPolicies?.get(view);
  if (!policy && view.__webuiTrustedTypesPolicyName !== undefined) {
    throw new Error('[WebUI] Duplicate Trusted Types runtime. Share one framework module across bootstrap and component bundles.');
  }
  return policy;
}

function validatePolicyName(name: string): void {
  if (typeof name !== 'string' || name.length === 0 || name === 'default') {
    throw new TypeError('[WebUI] Supply a nonempty Trusted Types policy name other than "default".');
  }
  for (let i = 0; i < name.length; i++) {
    const c = name.charCodeAt(i);
    if (
      (c >= 65 && c <= 90) || (c >= 97 && c <= 122) ||
      (c >= 48 && c <= 57) || c === 45 || c === 46 || c === 95
    ) continue;
    throw new TypeError('[WebUI] Policy names allow only ASCII letters, digits, ".", "_" and "-".');
  }
}

/** Record immutable compiler HTML during template normalization, never state updates. */
export function registerTrustedTemplateBlock(meta: TemplateBlockMeta): void {
  getDocumentPolicy(window)?.registerBlock(meta);
}

/** @internal Parse registered compiler HTML without exposing a trusted value. */
export function setTemplateContent(target: HTMLTemplateElement, meta: TemplateBlockMeta): void {
  const trust = getDocumentPolicy(window);
  if (trust) trust.setTemplateContent(target, meta);
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
    throw new TypeError('[WebUI] CSS data requires a script with type="importmap".');
  }
  const json = JSON.stringify({
    imports: { [specifier]: `data:text/css,${encodeURIComponent(css)}` },
  });
  const trust = getDocumentPolicy(view);
  if (trust) trust.setImportMap(script, json);
  else script.textContent = json;
}
