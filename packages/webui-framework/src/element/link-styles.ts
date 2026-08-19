// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  templateHtmlMayContainLink,
  getTemplateStylesheets,
  type TemplateStylesheetDescriptor,
} from '../template-content.js';
import type { TemplateBlockMeta, TemplateMeta } from '../template-types.js';

interface LinkStyleFailure {
  readonly href: string;
  readonly reason: string;
}

interface TemplateLinkStyleState {
  readonly descriptors: readonly TemplateStylesheetDescriptor[];
  preloads?: HTMLLinkElement[];
  preloadStartedAt?: number;
  ready: Promise<void>;
  results?: readonly (LinkStyleFailure | undefined)[];
  sheets?: CSSStyleSheet[];
}

interface SpeculativeStylesheetPreload {
  claimed: boolean;
  cleanupTimer?: ReturnType<typeof setTimeout>;
  readonly link: HTMLLinkElement;
  readonly ready: Promise<LinkStyleFailure | undefined>;
  readonly startedAt: number;
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
const NO_LINK_STYLE_STATE: TemplateLinkStyleState = {
  descriptors: NO_DESCRIPTORS,
  ready: READY,
};
const CSS_IMPORT_RULE = 3;
const STYLESHEET_PREPARATION_TIMEOUT_MS = 3_000;
const SPECULATIVE_PRELOAD_TTL_MS = 3_000;
const GUARD_CSS =
  '@layer{:host{transition:none!important;visibility:hidden!important}}';
const templateLinkStyles = new WeakMap<TemplateBlockMeta, TemplateLinkStyleState>();
const activeStyleMounts = new WeakMap<ShadowRoot, (discardRoot?: boolean) => void>();
let speculativeStylesheetPreloads:
  | Map<string, SpeculativeStylesheetPreload | undefined>
  | undefined;

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
  let pending: Promise<void>[] | undefined;
  for (let i = 0; i < names.length; i++) {
    const ready = prepareTemplateLinkStyles(templates[names[i]]);
    if (!ready) continue;
    (pending ??= []).push(ready);
  }
  return pending ? Promise.all(pending).then(ignorePromiseResults) : undefined;
}

/** Start compiler-owned component asset styles before importing the root module. */
export function preloadComponentAssetStyles(hrefs: readonly string[]): void {
  if (hrefs.length === 0 || !supportsConstructableStylesheets()) return;

  const preloads = (speculativeStylesheetPreloads ??= new Map());
  for (let i = 0; i < hrefs.length; i++) {
    if (hrefs[i].length === 0) continue;
    const href = resolveHref(hrefs[i]);
    if (preloads.has(href)) continue;

    const link = createStylesheetPreload(href);
    const startedAt = performanceNow();
    const preload: SpeculativeStylesheetPreload = {
      claimed: false,
      link,
      ready: waitForStylesheetPreload(link, href),
      startedAt,
    };
    preloads.set(href, preload);
    preload.cleanupTimer = setTimeout(() => {
      releaseStylesheetPreload(link);
    }, SPECULATIVE_PRELOAD_TTL_MS);
  }
}

/** Return whether a template can create a stylesheet link after bindings run. */
export function templateMayContainLinkStyles(meta: TemplateBlockMeta): boolean {
  return getTemplateLinkStyleState(meta) !== NO_LINK_STYLE_STATE;
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
  const mountStartedAt = performanceNow();
  const timingStartedAt = state.preloadStartedAt === undefined
    ? mountStartedAt
    : Math.min(state.preloadStartedAt, mountStartedAt);
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
    if (cancelled || failed || settled[index] !== 0) return;
    settled[index] = 1;
    remaining -= 1;
    removeLinkListeners(index);
    if (remaining === 0) finishMount(true);
  };

  const fail = (index: number): void => {
    if (cancelled || failed) return;
    failed = true;
    nativeOnly = true;
    removeAllLinkListeners();
    removeStylesheetPreloads(state);
    const result =
      state.descriptors.length === links.length
        ? state.results?.[index]
        : undefined;
    const href = result?.href || links[index].href;
    let reason = '';
    if (result?.reason) {
      reason = ` Constructable preparation failed: ${result.reason}.`;
    }
    console.error(
      `[WebUI] Stylesheet "${href}" failed to load for <${host.localName}>.${reason} ` +
      'The component will continue with its native stylesheet links and may be unstyled.',
    );
    finishMount(false);
  };

  const tryAdoptNativeSheets = (): void => {
    if (cancelled || nativeOnly) return;
    if (state.descriptors.length !== links.length) {
      nativeOnly = true;
      return;
    }

    let sheets = state.sheets;
    if (!sheets) {
      sheets = constructNativeStylesheets(
        links,
        state.descriptors,
        timingStartedAt,
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

  function finishMount(reconcileLinks: boolean): void {
    if (deferContent && !contentAppended) {
      const loadKeys = reconcileLinks ? captureNativeLoadKeys(links) : undefined;
      deferredCallbacks?.beforeAppend?.();
      if (loadKeys && rearmChangedLinks(loadKeys)) return;
    }
    const appended = appendDeferredContent();
    if (appended) {
      try {
        deferredCallbacks?.afterAppend?.();
      } finally {
        finishStyleMount();
      }
    } else {
      finishStyleMount();
    }
  }

  function finishStyleMount(): void {
    const hasAuthoredStyle =
      !nativeOnly && hasAuthorStyle(root, stagingRoot, guardStyle);
    releaseGuard?.();
    releaseGuard = undefined;
    removeStylesheetPreloads(state);
    clearActiveMount();
    if (
      !nativeOnly &&
      !hasAuthoredStyle &&
      state.descriptors.length === links.length
    ) {
      tryAdoptNativeSheets();
    }
  }

  const cancel = (discardRoot = false): void => {
    if (cancelled) return;
    cancelled = true;
    removeAllLinkListeners();
    releaseGuard?.();
    releaseGuard = undefined;
    removeStylesheetPreloads(state);
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
  const cached = templateLinkStyles.get(meta);
  if (cached) return cached;

  if (!templateHtmlMayContainLink(meta.h)) {
    templateLinkStyles.set(meta, NO_LINK_STYLE_STATE);
    return NO_LINK_STYLE_STATE;
  }

  const descriptors = getTemplateStylesheets(meta);
  if (!descriptors) {
    templateLinkStyles.set(meta, NO_LINK_STYLE_STATE);
    return NO_LINK_STYLE_STATE;
  }
  const state: TemplateLinkStyleState = {
    descriptors,
    ready: READY,
  };
  templateLinkStyles.set(meta, state);
  if (descriptors.length === 0) return state;

  if (!supportsConstructableStylesheets()) {
    state.results = fallbackResults(descriptors, 'Constructable stylesheets are unavailable');
    return state;
  }

  const pending = new Array<Promise<LinkStyleFailure | undefined>>(descriptors.length);
  for (let i = 0; i < descriptors.length; i++) {
    pending[i] = warmStylesheet(descriptors[i], state);
  }
  state.ready = Promise.all(pending).then(results => {
    state.results = results;
  });
  return state;
}

function supportsConstructableStylesheets(): boolean {
  return (
    typeof CSSStyleSheet === 'function' &&
    typeof ShadowRoot === 'function' &&
    'adoptedStyleSheets' in ShadowRoot.prototype &&
    typeof CSSStyleSheet.prototype.replaceSync === 'function' &&
    typeof document === 'object' &&
    document.head !== null
  );
}

function performanceNow(): number {
  return typeof performance === 'object' && typeof performance.now === 'function'
    ? performance.now()
    : Number.POSITIVE_INFINITY;
}

function removeStylesheetPreloads(state: TemplateLinkStyleState): void {
  const preloads = state.preloads;
  if (!preloads) return;
  state.preloads = undefined;
  void state.ready.then(() => {
    for (let i = 0; i < preloads.length; i++) {
      releaseStylesheetPreload(preloads[i]);
    }
  });
}

function warmStylesheet(
  descriptor: TemplateStylesheetDescriptor,
  state: TemplateLinkStyleState,
): Promise<LinkStyleFailure | undefined> {
  const href = resolveHref(descriptor.href);
  const fallbackReason = unsupportedLinkReason(descriptor);
  const speculative = speculativeStylesheetPreloads?.get(href);
  if (fallbackReason) {
    if (speculative && !speculative.claimed) {
      releaseStylesheetPreload(speculative.link);
    }
    return Promise.resolve({ href, reason: fallbackReason });
  }

  if (speculative && hasDefaultPreloadAttributes(descriptor)) {
    speculative.claimed = true;
    trackStylesheetPreload(state, speculative.link, speculative.startedAt);
    return speculative.ready;
  }
  if (speculative && !speculative.claimed) {
    releaseStylesheetPreload(speculative.link);
  }

  const preload = createStylesheetPreload(href, descriptor);
  const startedAt = performanceNow();
  trackStylesheetPreload(state, preload, startedAt);
  return waitForStylesheetPreload(preload, href);
}

function createStylesheetPreload(
  href: string,
  descriptor?: TemplateStylesheetDescriptor,
): HTMLLinkElement {
  const preload = document.createElement('link');
  preload.rel = 'preload';
  preload.as = 'style';
  preload.href = href;
  if (descriptor?.crossOrigin !== null && descriptor?.crossOrigin !== undefined) {
    preload.crossOrigin = descriptor.crossOrigin;
  }
  if (descriptor?.integrity) preload.integrity = descriptor.integrity;
  if (descriptor?.referrerPolicy) {
    preload.referrerPolicy = descriptor.referrerPolicy;
  }
  return preload;
}

function waitForStylesheetPreload(
  preload: HTMLLinkElement,
  href: string,
): Promise<LinkStyleFailure | undefined> {
  return new Promise(resolve => {
    let settled = false;
    const finish = (reason?: string): void => {
      if (settled) return;
      settled = true;
      clearTimeout(timeout);
      preload.onload = null;
      preload.onerror = null;
      resolve(reason ? { href, reason } : undefined);
    };
    const timeout = setTimeout(() => {
      finish(`preparation timed out after ${STYLESHEET_PREPARATION_TIMEOUT_MS}ms`);
    }, STYLESHEET_PREPARATION_TIMEOUT_MS);
    preload.onload = () => finish();
    preload.onerror = () => finish('stylesheet preload failed');
    try {
      document.head.appendChild(preload);
    } catch (error) {
      finish(errorMessage(error));
    }
  });
}

function trackStylesheetPreload(
  state: TemplateLinkStyleState,
  preload: HTMLLinkElement,
  startedAt: number,
): void {
  (state.preloads ??= []).push(preload);
  state.preloadStartedAt = state.preloadStartedAt === undefined
    ? startedAt
    : Math.min(state.preloadStartedAt, startedAt);
}

function releaseStylesheetPreload(preload: HTMLLinkElement): void {
  const speculative = speculativeStylesheetPreloads?.get(preload.href);
  if (speculative?.link === preload) {
    if (speculative.cleanupTimer !== undefined) {
      clearTimeout(speculative.cleanupTimer);
    }
    speculativeStylesheetPreloads?.delete(preload.href);
  }
  preload.remove();
}

function hasDefaultPreloadAttributes(
  descriptor: TemplateStylesheetDescriptor,
): boolean {
  return (
    descriptor.crossOrigin === null &&
    descriptor.integrity.length === 0 &&
    descriptor.referrerPolicy.length === 0
  );
}

function fallbackResults(
  descriptors: readonly TemplateStylesheetDescriptor[],
  reason: string,
): readonly LinkStyleFailure[] {
  const results = new Array<LinkStyleFailure>(descriptors.length);
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
): CSSStyleSheet[] | undefined {
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

  const rewritten = new Array<string>(rules.length);
  for (let i = 0; i < rules.length; i++) {
    const rule = rules[i];
    if (rule.type === CSS_IMPORT_RULE) return undefined;
    const cssText = rewriteCssUrls(rule.cssText, link.href);
    if (cssText === undefined) return undefined;
    rewritten[i] = cssText;
  }
  return rewritten.join('\n');
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
  sheets: CSSStyleSheet[],
): boolean {
  if (!('adoptedStyleSheets' in root)) return false;
  const current = root.adoptedStyleSheets;
  if (current.length === 0) {
    try {
      root.adoptedStyleSheets = sheets;
      return true;
    } catch {
      return false;
    }
  }
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

function ignorePromiseResults(): void {}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
