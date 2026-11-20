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

interface BrowserWindow extends Window {
  readonly trustedTypes?: BrowserTrustedTypes;
}

const compilerCapability = {};
let documentPolicies: WeakMap<Window, BrowserPolicy> | undefined;
let templateHTML: WeakMap<TemplateBlockMeta, string> | undefined;

function authorize(input: string, token: object): string {
  if (token !== compilerCapability) {
    throw new TypeError('[WebUI] Only the compiled-template runtime may use this policy.');
  }
  return input;
}

function getDocumentPolicy(view: BrowserWindow | null): BrowserPolicy | undefined {
  if (!view?.trustedTypes) return undefined;
  const existing = documentPolicies?.get(view);
  if (existing) return existing;

  let policy: BrowserPolicy;
  try {
    policy = view.trustedTypes.createPolicy('webui', {
      createHTML: authorize,
      createScript: authorize,
    });
  } catch (cause) {
    throw new Error('[WebUI] Could not create Trusted Types policy. Allow "webui" in CSP and share one framework module.', { cause });
  }
  (documentPolicies ??= new WeakMap()).set(view, policy);
  return policy;
}

/** Record immutable compiler HTML during template normalization, never state updates. */
export function registerTrustedTemplateBlock(meta: TemplateBlockMeta): void {
  const view: BrowserWindow = window;
  if (!view.trustedTypes) return;
  templateHTML ??= new WeakMap();
  if (!templateHTML.has(meta)) templateHTML.set(meta, meta.h);
}

/** @internal Parse registered compiler HTML without exposing a trusted value. */
export function setTemplateContent(target: Element, meta: TemplateBlockMeta): void {
  const policy = getTemplatePolicy(meta);
  target.innerHTML = policy?.createHTML(meta.h, compilerCapability) as string & TrustedValue ?? meta.h;
}

/** Preserve compiler registration and CSP policy checks without an HTML sink. */
export function validateTrustedTemplateBlock(meta: TemplateBlockMeta): void {
  getTemplatePolicy(meta);
}

function getTemplatePolicy(meta: TemplateBlockMeta): BrowserPolicy | undefined {
  const policy = getDocumentPolicy(window);
  if (!policy) return;
  const html = templateHTML?.get(meta);
  if (html === undefined || html !== meta.h) {
    throw new Error('[WebUI] Unregistered or modified template. Register immutable compiler output.');
  }
  return policy;
}

/**
 * @internal Install condition tables for templates being registered from a
 * trusted compiler response. The templates and closure sources are code, not
 * request state. Neither the policy nor a TrustedScript leaves this module.
 *
 * The document nonce satisfies CSP and the compiler-owned `webui` policy
 * satisfies Trusted Types, matching how SSR delivers the same closures.
 */
export function installCompilerConditions(closures: string, nonce: string): void {
  const script = document.createElement('script');
  if (nonce) script.nonce = nonce;
  setScriptContent(
    script,
    `Object.assign(window.__webui.templateFns??={},{${closures}});`,
    window,
  );
  document.head.appendChild(script);
  script.remove();
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
  setScriptContent(script, JSON.stringify({
    imports: { [specifier]: `data:text/css,${encodeURIComponent(css)}` },
  }), view);
}

function setScriptContent(script: HTMLScriptElement, source: string, view: Window | null): void {
  const policy = getDocumentPolicy(view);
  script.textContent = policy?.createScript(source, compilerCapability) as string & TrustedValue ?? source;
}
