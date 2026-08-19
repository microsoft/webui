// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  getTemplateStylesheets,
  type TemplateStylesheetDescriptor,
} from '../template-content.js';
import type { TemplateBlockMeta, TemplateMeta } from '../template-types.js';

interface PreparedLinkStyle {
  readonly href: string;
  readonly reason?: string;
}

interface TemplateLinkStyleState {
  readonly descriptors: readonly TemplateStylesheetDescriptor[];
  ready: Promise<void>;
  results?: readonly PreparedLinkStyle[];
  sheets?: readonly CSSStyleSheet[];
}

interface ElementBinding {
  readonly element: Element;
}

interface InstalledGuard {
  readonly effective: boolean;
  readonly release: () => void;
  readonly style: HTMLStyleElement;
}

interface DeferredContentCallbacks {
  readonly afterAppend?: () => void;
  readonly beforeAppend?: () => void;
  readonly hasAuthorHydrationLifecycle?: boolean;
}

const READY = Promise.resolve();
const NO_DESCRIPTORS: readonly TemplateStylesheetDescriptor[] = Object.freeze([]);
const STYLESHEET_PREPARATION_TIMEOUT_MS = 3_000;
const GUARD_CSS =
  '@layer{:host{transition:none!important;visibility:hidden!important}}';
const templateLinkStyles = new WeakMap<TemplateBlockMeta, TemplateLinkStyleState>();
const activeStyleMounts = new WeakMap<ShadowRoot, (discardRoot?: boolean) => void>();

/** Begin preparing Link-mode stylesheets for one compiled template. */
export function prepareTemplateLinkStyles(
  meta: TemplateBlockMeta,
): Promise<void> | undefined {
  const ready = getTemplateLinkStyleState(meta).ready;
  return ready === READY ? undefined : ready;
}

/** Prepare every Link-mode stylesheet in a template registration payload. */
export function prepareRegisteredLinkStyles(
  templates: Record<string, TemplateMeta>,
): Promise<void> | undefined {
  const names = Object.keys(templates);
  const pending = new Array<Promise<void>>(names.length);
  let count = 0;
  for (let i = 0; i < names.length; i++) {
    const ready = prepareTemplateLinkStyles(templates[names[i]]);
    if (!ready) continue;
    pending[count] = ready;
    count += 1;
  }
  if (count === 0) return undefined;
  pending.length = count;
  return Promise.all(pending).then(() => {});
}

/**
 * Adopt previously authorized Link-mode sheets or guard the original links
 * before a client shadow template is appended to its live root.
 */
export function installTemplateLinkStyles(
  host: HTMLElement,
  meta: TemplateMeta,
  shadowRoot: ShadowRoot | null,
  stagingRoot: ParentNode,
  bindings: readonly ElementBinding[],
  deferredCallbacks?: DeferredContentCallbacks,
): boolean {
  if (!shadowRoot) return false;
  const root = shadowRoot;

  const links = collectStylesheetLinks(stagingRoot);
  if (links.length === 0) return false;

  activeStyleMounts.get(root)?.();
  const state = getTemplateLinkStyleState(meta);
  let nativeOnly =
    hasBoundStylesheetLink(links, bindings) ||
    hasEventStylesheetLink(meta, state.descriptors);
  if (
    !nativeOnly &&
    state.sheets &&
    !deferredCallbacks?.hasAuthorHydrationLifecycle &&
    !hasAuthorStyle(root, stagingRoot)
  ) {
    if (state.sheets.length === links.length && adoptPreparedSheets(root, state.sheets)) {
      deactivatePromotedLinks(links);
      return false;
    }
    nativeOnly = true;
  }

  const settled = new Uint8Array(links.length);
  const loadHandlers = new Array<() => void>(links.length);
  const errorHandlers = new Array<() => void>(links.length);
  const mountStartedAt =
    typeof performance === 'object' && typeof performance.now === 'function'
      ? performance.now()
      : Number.POSITIVE_INFINITY;
  let remaining = 0;
  for (let i = 0; i < links.length; i++) {
    if (requiresNativeLoad(links[i])) {
      remaining += 1;
    } else {
      settled[i] = 1;
    }
  }
  if (remaining === 0) return false;
  let failed = false;
  let cancelled = false;
  let contentAppended = false;
  let deferContent = false;
  let guardStyle: HTMLStyleElement | undefined;
  let releaseGuard: (() => void) | undefined;

  const settle = (index: number): void => {
    if (cancelled || settled[index] !== 0) return;
    settled[index] = 1;
    remaining -= 1;
    removeLinkListeners(index);
    if (remaining === 0) {
      if (deferContent && !contentAppended) {
        const loadKeys = captureNativeLoadKeys(links);
        deferredCallbacks?.beforeAppend?.();
        if (rearmChangedLinks(loadKeys)) return;
      }
      const appended = appendDeferredContent();
      if (appended) {
        try {
          deferredCallbacks?.afterAppend?.();
        } finally {
          finishLoadedMount();
        }
      } else {
        finishLoadedMount();
      }
    }
  };

  const fail = (index: number): void => {
    if (cancelled || failed) return;
    failed = true;
    removeAllLinkListeners();
    const result = state.results?.[index];
    const href = result?.href || links[index].href;
    let reason = '';
    if (result?.reason) {
      reason = ` Constructable preparation failed: ${result.reason}.`;
    }
    console.error(
      `[WebUI] Stylesheet "${href}" failed to load for <${host.localName}>.${reason} ` +
      'The component remains hidden to prevent unstyled content.',
    );
  };

  const tryAdoptNativeSheets = (): void => {
    if (cancelled || nativeOnly) return;
    if (hasAuthorStyle(root, stagingRoot, guardStyle)) {
      nativeOnly = true;
      return;
    }
    if (state.descriptors.length !== links.length) {
      nativeOnly = true;
      return;
    }

    let sheets = state.sheets;
    if (!sheets) {
      sheets = constructNativeStylesheets(
        links,
        state.descriptors,
        mountStartedAt,
      );
      if (!sheets) {
        nativeOnly = true;
        return;
      }
      state.sheets = sheets;
    }

    if (!adoptPreparedSheets(root, sheets)) {
      nativeOnly = true;
      return;
    }

    deactivatePromotedLinks(links);
  };

  function removeLinkListeners(index: number): void {
    const load = loadHandlers[index];
    const error = errorHandlers[index];
    if (load) links[index].removeEventListener('load', load);
    if (error) links[index].removeEventListener('error', error);
  }

  function removeAllLinkListeners(): void {
    for (let i = 0; i < links.length; i++) removeLinkListeners(i);
  }

  function armLink(index: number): void {
    const onLoad = (): void => {
      if (settled[index] === 0) settle(index);
    };
    const onError = (): void => {
      if (cancelled) return;
      nativeOnly = true;
      fail(index);
    };
    loadHandlers[index] = onLoad;
    errorHandlers[index] = onError;
    links[index].addEventListener('load', onLoad, { once: true });
    links[index].addEventListener('error', onError, { once: true });
  }

  function rearmChangedLinks(previousKeys: readonly string[]): boolean {
    let rearmed = false;
    for (let i = 0; i < links.length; i++) {
      if (previousKeys[i] === nativeLoadKey(links[i])) continue;
      removeLinkListeners(i);
      if (!requiresNativeLoad(links[i])) {
        settled[i] = 1;
        continue;
      }
      settled[i] = 0;
      remaining += 1;
      armLink(i);
      rearmed = true;
    }
    return rearmed;
  }

  function clearActiveMount(): void {
    if (activeStyleMounts.get(root) === cancel) {
      activeStyleMounts.delete(root);
    }
  }

  function appendDeferredContent(): boolean {
    if (!deferContent || contentAppended) return false;
    contentAppended = true;
    while (stagingRoot.firstChild) {
      root.appendChild(stagingRoot.firstChild);
    }
    return true;
  }

  function finishLoadedMount(): void {
    tryAdoptNativeSheets();
    releaseGuard?.();
    releaseGuard = undefined;
    clearActiveMount();
  }

  const cancel = (discardRoot = false): void => {
    if (cancelled) return;
    cancelled = true;
    removeAllLinkListeners();
    releaseGuard?.();
    releaseGuard = undefined;
    if (discardRoot) root.replaceChildren();
    clearActiveMount();
  };
  activeStyleMounts.set(root, cancel);

  const guard = installGuard(host, root);
  guardStyle = guard.style;
  releaseGuard = guard.release;
  deferContent = !guard.effective;
  for (let i = 0; i < links.length; i++) {
    if (settled[i] !== 0) continue;
    armLink(i);
  }

  if (deferContent) {
    for (let i = 0; i < links.length; i++) root.appendChild(links[i]);
  }
  return deferContent;
}

/** Cancel a pending Link-mode mount before its template instance is destroyed. */
export function cancelTemplateLinkStyleMount(root: ShadowRoot): boolean {
  const cancel = activeStyleMounts.get(root);
  if (!cancel) return false;
  cancel(true);
  return true;
}

/** Return whether CSS text contains a top-level `@import` rule. */
export function hasTopLevelImport(cssText: string): boolean {
  let braceDepth = 0;
  let quote = 0;
  for (let i = 0; i < cssText.length; i++) {
    const code = cssText.charCodeAt(i);
    if (quote !== 0) {
      if (code === 92) {
        i += 1;
      } else if (code === quote) {
        quote = 0;
      }
      continue;
    }
    if (code === 34 || code === 39) {
      quote = code;
      continue;
    }
    if (code === 47 && cssText.charCodeAt(i + 1) === 42) {
      i = skipComment(cssText, i + 2);
      continue;
    }
    if (code === 123) {
      braceDepth += 1;
      continue;
    }
    if (code === 125) {
      if (braceDepth > 0) braceDepth -= 1;
      continue;
    }
    if (
      code === 64 &&
      braceDepth === 0 &&
      (
        matchesImportKeyword(cssText, i + 1) ||
        atKeywordContainsEscape(cssText, i + 1)
      )
    ) {
      return true;
    }
  }
  return false;
}

/**
 * Resolve CSS `url()` values against the external stylesheet location.
 *
 * Chromium currently ignores the `CSSStyleSheet` constructor's `baseURL`
 * option. Returning `undefined` selects the native link fallback when a value
 * cannot be rewritten without changing CSS token semantics.
 */
export function rewriteCssUrls(
  cssText: string,
  stylesheetHref: string,
): string | undefined {
  let quote = 0;
  let output = '';
  let copiedThrough = 0;
  for (let i = 0; i < cssText.length; i++) {
    const code = cssText.charCodeAt(i);
    if (quote !== 0) {
      if (code === 92) {
        i += 1;
      } else if (code === quote) {
        quote = 0;
      }
      continue;
    }
    if (code === 34 || code === 39) {
      quote = code;
      continue;
    }
    if (code === 47 && cssText.charCodeAt(i + 1) === 42) {
      i = skipComment(cssText, i + 2);
      continue;
    }
    if (code === 92) return undefined;
    if (
      matchesCssFunction(cssText, i, 'image-set') ||
      matchesCssFunction(cssText, i, '-webkit-image-set')
    ) {
      return undefined;
    }
    if (!matchesCssFunction(cssText, i, 'url')) continue;

    const token = readUrlToken(cssText, i + 3);
    if (!token) return undefined;
    const value = cssText.slice(token.valueStart, token.valueEnd);
    if (value.length > 0 && value.charCodeAt(0) !== 35) {
      let resolved: string;
      try {
        resolved = new URL(value, stylesheetHref).href;
      } catch {
        return undefined;
      }
      output += cssText.slice(copiedThrough, token.valueStart);
      output += token.quoted
        ? escapeCssString(resolved, token.quote)
        : `"${escapeCssString(resolved, 34)}"`;
      copiedThrough = token.valueEnd;
    }
    i = token.closeIndex;
  }
  if (copiedThrough === 0) return cssText;
  output += cssText.slice(copiedThrough);
  return output;
}

function getTemplateLinkStyleState(meta: TemplateBlockMeta): TemplateLinkStyleState {
  let state = templateLinkStyles.get(meta);
  if (state) return state;

  if (!mayContainLink(meta.h)) {
    state = {
      descriptors: NO_DESCRIPTORS,
      ready: READY,
      results: [],
    };
    templateLinkStyles.set(meta, state);
    return state;
  }

  const descriptors = getTemplateStylesheets(meta);
  state = {
    descriptors,
    ready: READY,
  };
  templateLinkStyles.set(meta, state);
  if (descriptors.length === 0) {
    state.results = [];
    return state;
  }

  if (!supportsConstructableStylesheets()) {
    state.results = fallbackResults(descriptors, 'Constructable stylesheets are unavailable');
    return state;
  }

  const pending = new Array<Promise<PreparedLinkStyle>>(descriptors.length);
  for (let i = 0; i < descriptors.length; i++) {
    pending[i] = warmStylesheet(descriptors[i]);
  }
  state.ready = Promise.all(pending).then(results => {
    state!.results = results;
  });
  return state;
}

function mayContainLink(html: string): boolean {
  for (let i = 0; i <= html.length - 5; i++) {
    if (html.charCodeAt(i) !== 60) continue;
    if (
      asciiLower(html.charCodeAt(i + 1)) === 108 &&
      asciiLower(html.charCodeAt(i + 2)) === 105 &&
      asciiLower(html.charCodeAt(i + 3)) === 110 &&
      asciiLower(html.charCodeAt(i + 4)) === 107
    ) {
      return true;
    }
  }
  return false;
}

function asciiLower(code: number): number {
  return code >= 65 && code <= 90 ? code + 32 : code;
}

function supportsConstructableStylesheets(): boolean {
  return (
    typeof CSSStyleSheet === 'function' &&
    typeof ShadowRoot === 'function' &&
    'adoptedStyleSheets' in ShadowRoot.prototype &&
    typeof CSSStyleSheet.prototype.replaceSync === 'function' &&
    typeof fetch === 'function' &&
    typeof AbortController === 'function'
  );
}

async function warmStylesheet(
  descriptor: TemplateStylesheetDescriptor,
): Promise<PreparedLinkStyle> {
  const href = resolveHref(descriptor.href);
  const fallbackReason = unsupportedLinkReason(descriptor);
  if (fallbackReason) return { href, reason: fallbackReason };

  const controller = new AbortController();
  let timedOut = false;
  const timeout = setTimeout(() => {
    timedOut = true;
    controller.abort();
  }, STYLESHEET_PREPARATION_TIMEOUT_MS);
  try {
    const init: RequestInit = {
      credentials: descriptor.crossOrigin?.toLowerCase() === 'use-credentials'
        ? 'include'
        : 'same-origin',
      signal: controller.signal,
    };
    if (descriptor.integrity) init.integrity = descriptor.integrity;
    if (
      descriptor.referrerPolicy &&
      isReferrerPolicy(descriptor.referrerPolicy)
    ) {
      init.referrerPolicy = descriptor.referrerPolicy;
    }
    const response = await fetch(href, init);
    if (!response.ok) {
      throw new Error(`HTTP ${response.status} ${response.statusText}`.trim());
    }
    if (!isCssResponse(response)) {
      return {
        href: response.url || href,
        reason: 'response Content-Type is not text/css',
      };
    }
    const responseHref = response.url || href;
    const cssText = await response.text();
    if (hasTopLevelImport(cssText)) {
      return { href, reason: 'top-level @import requires native link loading' };
    }
    const preparedCss = rewriteCssUrls(cssText, responseHref);
    if (preparedCss === undefined) {
      return {
        href,
        reason: 'relative CSS assets could not be safely resolved',
      };
    }
    return { href: responseHref };
  } catch (error) {
    return {
      href,
      reason: timedOut
        ? `preparation timed out after ${STYLESHEET_PREPARATION_TIMEOUT_MS}ms`
        : errorMessage(error),
    };
  } finally {
    clearTimeout(timeout);
  }
}

function fallbackResults(
  descriptors: readonly TemplateStylesheetDescriptor[],
  reason: string,
): readonly PreparedLinkStyle[] {
  const results = new Array<PreparedLinkStyle>(descriptors.length);
  for (let i = 0; i < descriptors.length; i++) {
    results[i] = {
      href: resolveHref(descriptors[i].href),
      reason,
    };
  }
  return results;
}

function unsupportedLinkReason(
  descriptor: TemplateStylesheetDescriptor,
): string | undefined {
  if (descriptor.disabled) return 'disabled stylesheets require native link loading';
  if (descriptor.title) return 'titled stylesheets require native link loading';
  if (descriptor.hasUnsupportedAttributes) {
    return 'unsupported link attributes require native link loading';
  }
  if (descriptor.hasInlineStyles) {
    return 'mixed inline and external shadow styles require native link cascade order';
  }
  if (descriptor.type && descriptor.type.toLowerCase() !== 'text/css') {
    return `stylesheet type "${descriptor.type}" requires native link loading`;
  }
  if (
    descriptor.referrerPolicy &&
    !isReferrerPolicy(descriptor.referrerPolicy)
  ) {
    return 'unsupported referrer policy requires native link loading';
  }
  return undefined;
}

function resolveHref(href: string): string {
  try {
    return new URL(href, document.baseURI).href;
  } catch {
    return href;
  }
}

function collectStylesheetLinks(root: ParentNode): HTMLLinkElement[] {
  const candidates = root.querySelectorAll<HTMLLinkElement>('link[rel]');
  const links: HTMLLinkElement[] = [];
  for (let i = 0; i < candidates.length; i++) {
    const link = candidates[i];
    if (
      link.relList.contains('stylesheet') &&
      !link.relList.contains('alternate') &&
      link.hasAttribute('href')
    ) {
      links.push(link);
    }
  }
  return links;
}

function requiresNativeLoad(link: HTMLLinkElement): boolean {
  if (
    !link.relList.contains('stylesheet') ||
    link.relList.contains('alternate') ||
    !link.hasAttribute('href')
  ) {
    return false;
  }
  if (link.disabled) return false;
  const type = link.type.toLowerCase();
  return type.length === 0 || type === 'text/css';
}

function captureNativeLoadKeys(
  links: readonly HTMLLinkElement[],
): readonly string[] {
  const keys = new Array<string>(links.length);
  for (let i = 0; i < links.length; i++) keys[i] = nativeLoadKey(links[i]);
  return keys;
}

function nativeLoadKey(link: HTMLLinkElement): string {
  return [
    link.disabled ? '1' : '0',
    link.rel,
    link.href,
    link.type,
    link.crossOrigin ?? '',
    link.integrity,
    link.referrerPolicy,
  ].join('\0');
}

function deactivatePromotedLinks(links: readonly HTMLLinkElement[]): void {
  for (let i = 0; i < links.length; i++) links[i].disabled = true;
}

function hasBoundStylesheetLink(
  links: readonly HTMLLinkElement[],
  bindings: readonly ElementBinding[],
): boolean {
  for (let i = 0; i < bindings.length; i++) {
    const element = bindings[i].element;
    for (let linkIndex = 0; linkIndex < links.length; linkIndex++) {
      if (element === links[linkIndex]) return true;
    }
  }
  return false;
}

function hasEventStylesheetLink(
  meta: TemplateMeta,
  descriptors: readonly TemplateStylesheetDescriptor[],
): boolean {
  const groups = meta.eg;
  if (!groups) return false;
  for (let groupIndex = 0; groupIndex < groups.length; groupIndex++) {
    const eventBindings = groups[groupIndex][1];
    for (let bindingIndex = 0; bindingIndex < eventBindings.length; bindingIndex++) {
      const target = eventBindings[bindingIndex][2];
      for (let descriptorIndex = 0; descriptorIndex < descriptors.length; descriptorIndex++) {
        if (descriptors[descriptorIndex].elementIndex === target) return true;
      }
    }
  }
  return false;
}

function constructNativeStylesheets(
  links: readonly HTMLLinkElement[],
  descriptors: readonly TemplateStylesheetDescriptor[],
  mountStartedAt: number,
): readonly CSSStyleSheet[] | undefined {
  const sheets = new Array<CSSStyleSheet>(links.length);
  for (let i = 0; i < links.length; i++) {
    if (unsupportedLinkReason(descriptors[i])) return undefined;
    const cssText = readNativeStylesheet(links[i], mountStartedAt);
    if (cssText === undefined) return undefined;

    const options: CSSStyleSheetInit = { baseURL: links[i].href };
    if (descriptors[i].media) options.media = descriptors[i].media;
    try {
      const sheet = new CSSStyleSheet(options);
      sheet.replaceSync(cssText);
      sheets[i] = sheet;
    } catch {
      return undefined;
    }
  }
  return sheets;
}

function readNativeStylesheet(
  link: HTMLLinkElement,
  mountStartedAt: number,
): string | undefined {
  if (!hasUnredirectedNativeTiming(link, mountStartedAt)) return undefined;
  const sheet = link.sheet;
  if (!sheet) return undefined;

  let rules: CSSRuleList;
  try {
    rules = sheet.cssRules;
  } catch {
    return undefined;
  }

  let cssText = '';
  for (let i = 0; i < rules.length; i++) {
    if (i !== 0) cssText += '\n';
    cssText += rules[i].cssText;
  }
  if (hasTopLevelImport(cssText)) return undefined;
  return rewriteCssUrls(cssText, link.href);
}

function hasUnredirectedNativeTiming(
  link: HTMLLinkElement,
  mountStartedAt: number,
): boolean {
  if (
    typeof performance !== 'object' ||
    typeof performance.getEntriesByName !== 'function'
  ) {
    return false;
  }

  const entries = performance.getEntriesByName(link.href, 'resource');
  for (let i = entries.length - 1; i >= 0; i--) {
    const entry = entries[i] as PerformanceResourceTiming;
    if (
      entry.initiatorType !== 'link' ||
      entry.startTime + 1 < mountStartedAt
    ) {
      continue;
    }
    if (entry.redirectStart !== 0 || entry.redirectEnd !== 0) return false;
    if (entry.workerStart !== 0) return false;
    const status = (entry as PerformanceResourceTiming & {
      readonly responseStatus?: number;
    }).responseStatus;
    if (typeof status === 'number' && status !== 0) {
      return status >= 200 && status < 300;
    }
    return (
      entry.transferSize > 0 ||
      entry.encodedBodySize > 0 ||
      entry.decodedBodySize > 0
    );
  }
  return false;
}

function adoptPreparedSheets(
  root: ShadowRoot,
  sheets: readonly CSSStyleSheet[],
): boolean {
  if (!('adoptedStyleSheets' in root)) return false;
  const current = root.adoptedStyleSheets;
  let retained = 0;
  for (let i = 0; i < current.length; i++) {
    let prepared = false;
    for (let sheetIndex = 0; sheetIndex < sheets.length; sheetIndex++) {
      if (current[i] === sheets[sheetIndex]) {
        prepared = true;
        break;
      }
    }
    if (!prepared) retained += 1;
  }
  const next = new Array<CSSStyleSheet>(sheets.length + retained);
  for (let i = 0; i < sheets.length; i++) next[i] = sheets[i];
  let index = sheets.length;
  for (let i = 0; i < current.length; i++) {
    let prepared = false;
    for (let sheetIndex = 0; sheetIndex < sheets.length; sheetIndex++) {
      if (current[i] === sheets[sheetIndex]) {
        prepared = true;
        break;
      }
    }
    if (!prepared) {
      next[index] = current[i];
      index += 1;
    }
  }
  try {
    root.adoptedStyleSheets = next;
    return true;
  } catch {
    return false;
  }
}

function hasAuthorStyle(
  root: ShadowRoot,
  stagingRoot: ParentNode,
  guard?: HTMLStyleElement,
): boolean {
  const mounted = root.querySelectorAll('style');
  for (let i = 0; i < mounted.length; i++) {
    if (mounted[i] !== guard) return true;
  }
  return stagingRoot.querySelector('style') !== null;
}

function installGuard(host: HTMLElement, root: ShadowRoot): InstalledGuard {
  const style = document.createElement('style');
  const nonce = window.__webui?.nonce;
  if (typeof nonce === 'string' && nonce) style.nonce = nonce;
  style.textContent = GUARD_CSS;
  root.prepend(style);
  let effective = false;
  try {
    effective = (style.sheet?.cssRules.length ?? 0) > 0;
  } catch {
    effective = false;
  }
  const previousTransitionValue =
    host.style.getPropertyValue('transition-property');
  const previousTransitionPriority =
    host.style.getPropertyPriority('transition-property');
  const previousValue = host.style.getPropertyValue('visibility');
  const previousPriority = host.style.getPropertyPriority('visibility');
  host.style.setProperty('transition-property', 'none', 'important');
  host.style.setProperty('visibility', 'hidden', 'important');
  return {
    effective,
    release: () => {
      style.remove();
      if (
        host.style.getPropertyValue('visibility') === 'hidden' &&
        host.style.getPropertyPriority('visibility') === 'important'
      ) {
        if (previousValue) {
          host.style.setProperty('visibility', previousValue, previousPriority);
        } else {
          host.style.removeProperty('visibility');
        }
      }
      if (
        host.style.getPropertyValue('transition-property') === 'none' &&
        host.style.getPropertyPriority('transition-property') === 'important'
      ) {
        if (previousTransitionValue) {
          host.style.setProperty(
            'transition-property',
            previousTransitionValue,
            previousTransitionPriority,
          );
        } else {
          host.style.removeProperty('transition-property');
        }
      }
    },
    style,
  };
}

function skipComment(cssText: string, start: number): number {
  for (let i = start; i < cssText.length - 1; i++) {
    if (cssText.charCodeAt(i) === 42 && cssText.charCodeAt(i + 1) === 47) {
      return i + 1;
    }
  }
  return cssText.length;
}

function matchesImportKeyword(cssText: string, start: number): boolean {
  const keyword = 'import';
  if (start + keyword.length > cssText.length) return false;
  for (let i = 0; i < keyword.length; i++) {
    let code = cssText.charCodeAt(start + i);
    if (code >= 65 && code <= 90) code += 32;
    if (code !== keyword.charCodeAt(i)) return false;
  }
  const next = cssText.charCodeAt(start + keyword.length);
  return !isCssNameCode(next);
}

interface UrlToken {
  readonly closeIndex: number;
  readonly quote: number;
  readonly quoted: boolean;
  readonly valueEnd: number;
  readonly valueStart: number;
}

function readUrlToken(cssText: string, openIndex: number): UrlToken | undefined {
  let cursor = openIndex + 1;
  while (isCssWhitespace(cssText.charCodeAt(cursor))) cursor += 1;
  const quote = cssText.charCodeAt(cursor);
  if (quote === 34 || quote === 39) {
    const valueStart = cursor + 1;
    cursor = valueStart;
    while (cursor < cssText.length) {
      const code = cssText.charCodeAt(cursor);
      if (code === 92) return undefined;
      if (code === quote) break;
      cursor += 1;
    }
    if (cursor >= cssText.length) return undefined;
    const valueEnd = cursor;
    cursor += 1;
    while (isCssWhitespace(cssText.charCodeAt(cursor))) cursor += 1;
    if (cssText.charCodeAt(cursor) !== 41) return undefined;
    return {
      closeIndex: cursor,
      quote,
      quoted: true,
      valueEnd,
      valueStart,
    };
  }

  const valueStart = cursor;
  while (cursor < cssText.length && cssText.charCodeAt(cursor) !== 41) {
    const code = cssText.charCodeAt(cursor);
    if (
      code === 34 ||
      code === 39 ||
      code === 40 ||
      code === 92 ||
      (code === 47 && cssText.charCodeAt(cursor + 1) === 42)
    ) {
      return undefined;
    }
    cursor += 1;
  }
  if (cursor >= cssText.length) return undefined;
  let valueEnd = cursor;
  while (
    valueEnd > valueStart &&
    isCssWhitespace(cssText.charCodeAt(valueEnd - 1))
  ) {
    valueEnd -= 1;
  }
  return {
    closeIndex: cursor,
    quote: 0,
    quoted: false,
    valueEnd,
    valueStart,
  };
}

function matchesCssFunction(
  cssText: string,
  start: number,
  name: string,
): boolean {
  if (start > 0 && isCssNameCode(cssText.charCodeAt(start - 1))) return false;
  if (start + name.length >= cssText.length) return false;
  for (let i = 0; i < name.length; i++) {
    let code = cssText.charCodeAt(start + i);
    if (code >= 65 && code <= 90) code += 32;
    if (code !== name.charCodeAt(i)) return false;
  }

  return cssText.charCodeAt(start + name.length) === 40;
}

function atKeywordContainsEscape(cssText: string, start: number): boolean {
  for (let i = start; i < cssText.length; i++) {
    const code = cssText.charCodeAt(i);
    if (code === 92) return true;
    if (!isCssNameCode(code)) return false;
  }
  return false;
}

function isCssWhitespace(code: number): boolean {
  return code === 9 || code === 10 || code === 12 || code === 13 || code === 32;
}

function escapeCssString(value: string, quote: number): string {
  let escaped = '';
  let copiedThrough = 0;
  for (let i = 0; i < value.length; i++) {
    const code = value.charCodeAt(i);
    if (code !== quote && code !== 92) continue;
    escaped += value.slice(copiedThrough, i);
    escaped += code === 92 ? '\\\\' : `\\${String.fromCharCode(code)}`;
    copiedThrough = i + 1;
  }
  return copiedThrough === 0
    ? value
    : escaped + value.slice(copiedThrough);
}

function isCssNameCode(code: number): boolean {
  return (
    (code >= 48 && code <= 57) ||
    (code >= 65 && code <= 90) ||
    (code >= 97 && code <= 122) ||
    code === 45 ||
    code === 95 ||
    code >= 128
  );
}

function isReferrerPolicy(value: string): value is ReferrerPolicy {
  return (
    value === 'no-referrer' ||
    value === 'no-referrer-when-downgrade' ||
    value === 'origin' ||
    value === 'origin-when-cross-origin' ||
    value === 'same-origin' ||
    value === 'strict-origin' ||
    value === 'strict-origin-when-cross-origin' ||
    value === 'unsafe-url'
  );
}

function isCssResponse(response: Response): boolean {
  const contentType = response.headers.get('content-type');
  if (!contentType) return false;
  const separator = contentType.indexOf(';');
  const mime = (separator < 0 ? contentType : contentType.slice(0, separator))
    .trim()
    .toLowerCase();
  return mime === 'text/css';
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
