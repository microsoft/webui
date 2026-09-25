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
  readonly policy: BrowserPolicy;
  readonly capability: object;
}

interface BrowserWindow extends Window {
  readonly trustedTypes?: BrowserTrustedTypes;
}

let documentPolicies: WeakMap<Window, CompilerPolicy> | undefined;
let templateHTML: WeakMap<TemplateBlockMeta, string> | undefined;

function getDocumentPolicy(view: BrowserWindow | null): CompilerPolicy | undefined {
  if (!view?.trustedTypes) return undefined;
  const existing = documentPolicies?.get(view);
  if (existing) return existing;

  const capability = {};
  const authorize = (input: string, token: object): string => {
    if (token !== capability) {
      throw new TypeError('[WebUI] Only the compiled-template runtime may use this Trusted Types policy.');
    }
    return input;
  };
  let policy: BrowserPolicy;
  try {
    policy = view.trustedTypes.createPolicy('webui', {
      createHTML: authorize,
      createScript: authorize,
    });
  } catch (cause) {
    throw new Error('[WebUI] Cannot create Trusted Types policy "webui". Allow "webui" in CSP, share one framework module, and do not pre-create the policy.', { cause });
  }
  const trust = { policy, capability };
  (documentPolicies ??= new WeakMap()).set(view, trust);
  return trust;
}

/** Record immutable compiler HTML during template normalization, never state updates. */
export function registerTrustedTemplateBlock(meta: TemplateBlockMeta): void {
  const view: BrowserWindow = window;
  if (!view.trustedTypes) return;
  templateHTML ??= new WeakMap();
  if (!templateHTML.has(meta)) templateHTML.set(meta, meta.h);
}

/** @internal Parse registered compiler HTML without exposing a trusted value. */
export function setTemplateContent(target: HTMLTemplateElement, meta: TemplateBlockMeta): void {
  const trust = getDocumentPolicy(window);
  if (!trust) {
    target.innerHTML = meta.h;
    return;
  }
  const html = templateHTML?.get(meta);
  if (html === undefined || html !== meta.h) {
    throw new Error('[WebUI] Unregistered or modified compiled template. Register only immutable compiler output.');
  }
  target.innerHTML = trust.policy.createHTML(html, trust.capability) as string & TrustedValue;
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
  if (trust) script.textContent = trust.policy.createScript(json, trust.capability) as string & TrustedValue;
  else script.textContent = json;
}
