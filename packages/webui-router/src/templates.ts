// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Template & CSS registration — shared by partial navigation and
 * `ensureLoaded()`.
 *
 * This module stores templates and announces that new WebUI template data
 * exists. The router stays framework-independent; optional runtimes decide
 * whether a registered template needs a compiler-owned host.
 */

import type { ComponentStyleResource, ComponentStyles } from './types.js';

/** Shared event name understood by optional framework runtimes. */
const TEMPLATES_REGISTERED_EVENT = 'webui:templates-registered';
const READINESS_COMPLETE = (): true => true;
const IGNORE_READINESS_RESULTS = (): void => {};

/**
 * Register templates + inject CSS from a server response.
 * Shared by fetchPartial and fetchComponentTemplates.
 *
 * WebUI JSON template payloads are stored directly. FAST string templates are
 * materialized as DOM.
 */
export function registerTemplatesAndStyles(
  data: {
    templates?: Record<string, unknown>;
    templateFunctions?: Record<string, string>;
    componentStyles?: ComponentStyles;
    inventory?: string;
  },
  nonce: string,
  updateInventory: (inv: string) => void,
): Promise<void> | undefined;
export function registerTemplatesAndStyles(
  data: {
    templates?: Record<string, unknown>;
    templateFunctions?: Record<string, string>;
    componentStyles?: ComponentStyles;
    inventory?: string;
  },
  nonce: string,
  injectedStyles: Set<string>,
  updateInventory: (inv: string) => void,
): Promise<void> | undefined;
export function registerTemplatesAndStyles(
  data: {
    templates?: Record<string, unknown>;
    templateFunctions?: Record<string, string>;
    componentStyles?: ComponentStyles;
    inventory?: string;
  },
  nonce: string,
  injectedStylesOrUpdate: Set<string> | ((inv: string) => void),
  maybeUpdateInventory?: (inv: string) => void,
): Promise<void> | undefined {
  const injectedStyles = injectedStylesOrUpdate instanceof Set
    ? injectedStylesOrUpdate
    : new Set<string>();
  const updateInventory = typeof injectedStylesOrUpdate === 'function'
    ? injectedStylesOrUpdate
    : maybeUpdateInventory ?? (() => {});
  const componentStyles = data.componentStyles
    ? validateComponentStyles(data.componentStyles)
    : undefined;
  validateTemplatePayload(data.templates);
  const styleRegistration = componentStyles
    ? registerComponentStyleCatalog(componentStyles)
    : undefined;
  if (data.inventory) {
    updateInventory(data.inventory);
  }

  let executableTemplateBody = '';
  let registeredTemplates: Record<string, unknown> | undefined;
  // 1. Template closures: execute only the component-local condition arrays.
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

  if (executableTemplateBody) {
    const script = document.createElement('script');
    if (nonce) script.nonce = nonce;
    script.textContent = `(function(){${executableTemplateBody}})();`;
    document.head.appendChild(script);
    document.head.removeChild(script);
  }

  // 2. Template metadata is published only after resources and closures.
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
          throw new Error(`[Router] Unsupported executable template payload for ${tag}.`);
        }
      } else {
        w.__webui.templates[tag] = template;
        if (!registeredTemplates) registeredTemplates = {};
        registeredTemplates[tag] = template;
      }
    }
  }

  // The bridge already accepted this exact payload. Only fallback listeners
  // need styles included with the template announcement.
  const readiness = notifyTemplatesRegistered(
    registeredTemplates,
    styleRegistration?.bridged ? undefined : componentStyles,
  );
  if (!styleRegistration?.ready) return readiness;
  if (!readiness) return styleRegistration.ready;
  return Promise.all([styleRegistration.ready, readiness]).then(IGNORE_READINESS_RESULTS);
}

/** Register initial bootstrap styles before announcing already-published templates. */
export function registerInitialTemplatesAndStyles(
  templates: Record<string, unknown>,
  componentStyles: ComponentStyles,
): void {
  const styles = validateComponentStyles(componentStyles);
  validateTemplatePayload(templates);
  const styleRegistration = registerComponentStyleCatalog(styles);
  notifyTemplatesRegistered(
    templates,
    styleRegistration.bridged ? undefined : styles,
  );
}

function registerComponentStyleCatalog(componentStyles: ComponentStyles): {
  bridged: boolean;
  ready?: Promise<void>;
} {
  const register = window.__webuiRegisterComponentStyles;
  if (register) {
    return { bridged: true, ready: register(componentStyles) };
  }
  const w = window as Window;
  if (!w.__webui) w.__webui = {};
  w.__webui.componentStyles = mergeFallbackComponentStyles(
    w.__webui.componentStyles,
    componentStyles,
  );
  return { bridged: false };
}

function mergeFallbackComponentStyles(
  current: ComponentStyles | undefined,
  next: ComponentStyles,
): ComponentStyles {
  if (!current) return next;
  if (current.strategy !== next.strategy) {
    throw new Error('[Router] Conflicting componentStyles strategies.');
  }
  const resources = { ...current.resources };
  const closures = { ...current.closures };
  for (const id of Object.keys(next.resources)) {
    const existing = resources[id];
    if (existing && !sameComponentStyleResource(existing, next.resources[id])) {
      throw new Error(`[Router] Conflicting component style resource "${id}".`);
    }
    resources[id] = next.resources[id];
  }
  for (const root of Object.keys(next.closures)) {
    const existing = closures[root];
    if (existing && (
      existing.length !== next.closures[root].length ||
      existing.some((id, index) => id !== next.closures[root][index])
    )) {
      throw new Error(`[Router] Conflicting component style closure "${root}".`);
    }
    closures[root] = next.closures[root];
  }
  return {
    version: 1,
    strategy: next.strategy,
    resources,
    closures,
  };
}

function sameComponentStyleResource(
  current: ComponentStyleResource,
  next: ComponentStyleResource,
): boolean {
  if (current === next) return true;
  const currentMembers = current.members;
  const nextMembers = next.members;
  if (
    currentMembers?.length !== nextMembers?.length ||
    currentMembers?.some((member, index) => member !== nextMembers?.[index])
  ) {
    return false;
  }
  switch (current.kind) {
    case 'link':
      return next.kind === 'link' && current.href === next.href;
    case 'style':
      return next.kind === 'style' && current.css === next.css;
    case 'module':
      return next.kind === 'module' &&
        current.specifier === next.specifier &&
        current.css === next.css;
  }
}

function validateTemplatePayload(
  templates: Record<string, unknown> | undefined,
): void {
  if (!templates) return;
  for (const tag of Object.keys(templates)) {
    const template = templates[tag];
    if (typeof template === 'string' && !template.startsWith('<')) {
      throw new Error(`[Router] Unsupported executable template payload for ${tag}.`);
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

/** Await optional runtime readiness while allowing a stale navigation to stop promptly. */
export function waitForTemplateReadiness(
  ready: Promise<void>,
  signal?: AbortSignal,
): Promise<boolean> {
  if (!signal) return ready.then(READINESS_COMPLETE);
  if (signal.aborted) return Promise.resolve(false);
  return new Promise<boolean>((resolve, reject) => {
    const onAbort = (): void => {
      signal.removeEventListener('abort', onAbort);
      resolve(false);
    };
    signal.addEventListener('abort', onAbort, { once: true });
    ready.then(
      () => {
        signal.removeEventListener('abort', onAbort);
        resolve(true);
      },
      error => {
        signal.removeEventListener('abort', onAbort);
        reject(error);
      },
    );
  });
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
  injectedStylesOrUpdate: Set<string> | ((inv: string) => void),
  maybeUpdateInventory?: (inv: string) => void,
): Promise<void> {
  const url = `${templateEndpoint}?t=${tags.join(',')}&inv=${encodeURIComponent(inventoryHex)}`;
  const resp = await fetch(url);
  if (!resp.ok) {
    throw new Error(`[Router] ensureLoaded failed: ${resp.status} ${resp.statusText}`);
  }
  const data = await resp.json();

  // Register using the same pipeline as partial navigation
  const ready = injectedStylesOrUpdate instanceof Set
    ? registerTemplatesAndStyles(
      data,
      nonce,
      injectedStylesOrUpdate,
      maybeUpdateInventory ?? (() => {}),
    )
    : registerTemplatesAndStyles(data, nonce, injectedStylesOrUpdate);
  if (ready) await ready;
}

/** Announce newly registered WebUI templates. */
export function notifyTemplatesRegistered(
  templates: Record<string, unknown> | undefined,
  componentStyles?: ComponentStyles,
): Promise<void> | undefined {
  if (
    !templates ||
    typeof window === 'undefined' ||
    typeof CustomEvent !== 'function' ||
    typeof window.dispatchEvent !== 'function'
  ) {
    return undefined;
  }

  let pending: PromiseLike<unknown>[] | undefined;
  let accepting = true;
  const waitUntil = (promise: PromiseLike<unknown>): void => {
    if (!accepting) {
      throw new Error(
        '[Router] webui:templates-registered waitUntil() must be called during event dispatch.',
      );
    }
    (pending ??= []).push(promise);
  };
  try {
    window.dispatchEvent(new CustomEvent(TEMPLATES_REGISTERED_EVENT, {
      detail: { templates, componentStyles, waitUntil },
    }));
  } finally {
    accepting = false;
  }
  return pending
    ? Promise.all(pending).then(IGNORE_READINESS_RESULTS)
    : undefined;
}

function validateComponentStyles(value: unknown): ComponentStyles {
  if (
    !value ||
    typeof value !== 'object' ||
    Array.isArray(value) ||
    (value as { version?: unknown }).version !== 1
  ) {
    throw new Error('[Router] componentStyles must use version 1.');
  }
  const styles = value as unknown as ComponentStyles;
  if (
    styles.strategy !== 'link' &&
    styles.strategy !== 'style' &&
    styles.strategy !== 'module'
  ) {
    throw new Error('[Router] Invalid componentStyles strategy.');
  }
  if (
    !styles.resources ||
    typeof styles.resources !== 'object' ||
    Array.isArray(styles.resources) ||
    !styles.closures ||
    typeof styles.closures !== 'object' ||
    Array.isArray(styles.closures)
  ) {
    throw new Error('[Router] componentStyles must contain resources and closures.');
  }
  for (const id of Object.keys(styles.resources)) {
    const resource = styles.resources[id];
    if (!id || !resource || resource.kind !== styles.strategy) {
      throw new Error(`[Router] Invalid component style resource "${id}".`);
    }
    if (
      (resource.kind === 'link' && (!resource.href || typeof resource.href !== 'string')) ||
      (resource.kind === 'style' && typeof resource.css !== 'string') ||
      (resource.kind === 'module' && (
        !resource.specifier || typeof resource.specifier !== 'string' ||
        typeof resource.css !== 'string'
      ))
    ) {
      throw new Error(`[Router] Invalid component style resource "${id}".`);
    }
    if (
      resource.members !== undefined &&
      (
        !Array.isArray(resource.members) ||
        resource.members.length < 2 ||
        resource.members.some(member => typeof member !== 'string' || !member) ||
        new Set(resource.members).size !== resource.members.length
      )
    ) {
      throw new Error(`[Router] Invalid component style resource members "${id}".`);
    }
  }
  for (const root of Object.keys(styles.closures)) {
    const closure = styles.closures[root];
    if (!root || !Array.isArray(closure) || closure.some(id => typeof id !== 'string' || !id)) {
      throw new Error(`[Router] Invalid component style closure "${root}".`);
    }
  }
  return styles;
}
