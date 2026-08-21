// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { installComponentLinkStyles } from './link-styles.js';

// `__WEBUI_DEV__` is a compile-time constant a bundler folds to a literal, so
// the `if (DEV)` diagnostics below become dead code in production builds. See
// the longer note in `template-element.ts`; the flag is declared module-locally
// on purpose, because esbuild does not inline an imported constant.
declare const __WEBUI_DEV__: boolean;
const DEV: boolean = typeof __WEBUI_DEV__ === 'undefined' || __WEBUI_DEV__;

/** One addressable component stylesheet definition. */
export type ComponentStyleResource = (
  | { kind: 'link'; href: string }
  | { kind: 'style'; css: string }
  | { kind: 'module'; specifier: string; css: string }
) & {
  /** Component resource IDs whose rules this bundled resource covers. */
  members?: string[];
};

/** Versioned stylesheet catalog shared by server and client registration paths. */
export interface ComponentStyles {
  version: 1;
  strategy: 'link' | 'style' | 'module';
  resources: Record<string, ComponentStyleResource>;
  closures: Record<string, string[]>;
}

interface DocumentCatalog {
  readonly resources: Map<string, ComponentStyleResource>;
  readonly closures: Map<string, readonly string[]>;
}

interface SsrStyleMarkerState {
  moduleIds?: string[];
  moduleElements?: Map<string, Element>;
}

/**
 * Stylesheet management for WebUI components.
 *
 * Three CSS strategies are supported:
 *
 * - **Link**: The first client shadow mount authorizes external CSS through its
 *   native links, then promotes their CSSOM into shared constructable sheets.
 *   Warm mounts adopt those sheets synchronously; guarded native links remain
 *   the compatibility and failure path.
 *
 * - **Style**: Inline `<style>` tags inside each shadow template.
 *
 * - **Module**: Uses CSS Modules registered via Import Maps. During SSR, the
 *   handler emits a `<script type="importmap">{"imports":{"<tag>":"data:text/css,..."}}</script>`
 *   in each rendered component's light DOM. The browser registers the
 *   stylesheet globally under `<tag>` and automatically adopts it via
 *   `shadowrootadoptedstylesheets` on declarative shadow roots.
 *
 *   During SPA navigation, the router appends new importmap script tags to
 *   `<head>` via `templateStyles[]`. The framework uses
 *   `import(specifier, { with: { type: "css" } })` to retrieve the browser's
 *   registered CSSStyleSheet and adopts it onto the shadow root. This is a
 *   direct hash-map lookup in the browser's module registry - no DOM queries,
 *   no manual CSSStyleSheet construction.
 *
 * For light DOM components (no shadow root), Module mode injects a `<style>`
 * element in `<head>`, deduplicated by `headInjected`.
 */

/**
 * Cached anchor elements for one CSS tree's direct resource markers.
 *
 * `count` is the `childElementCount` observed when `markers` was last known to
 * be accurate. Every install we perform updates both fields, so the common case
 * — repeated closure installs into a head we are the only writer of — never
 * rescans. A count that no longer matches means somebody else mutated the tree,
 * and the scan is redone.
 */
interface ResourceMarkerCache {
  count: number;
  markers: Map<string, Element>;
}

/**
 * Module adoption bookkeeping for one CSS tree.
 *
 * `order` is the closure request order, so a descendant closure whose network
 * load resolves first cannot adopt ahead of the caller that reserved earlier.
 */
interface ModuleAdoptionState {
  readonly order: string[];
  readonly reserved: Set<string>;
  readonly sheets: Map<string, CSSStyleSheet>;
}

type StyleTarget = Document | ShadowRoot;
type CssModule = { default: CSSStyleSheet };
type CssModuleLoader = (specifier: string) => Promise<CssModule>;

interface WebUIStyleGlobal {
  nonce?: string;
  styles?: string[];
  [key: string]: unknown;
}

const catalogs = new WeakMap<Document, DocumentCatalog>();
const installed = new WeakMap<StyleTarget, Set<string>>();
const completedClosures = new WeakMap<StyleTarget, Set<string>>();
const pending = new WeakMap<StyleTarget, Map<string, Promise<CSSStyleSheet>>>();
const ssrMarkers = new WeakMap<StyleTarget, SsrStyleMarkerState>();
const resourceMarkers = new WeakMap<StyleTarget, ResourceMarkerCache>();
const moduleAdoption = new WeakMap<StyleTarget, ModuleAdoptionState>();
const moduleImportsInstalled = new WeakMap<Document, Set<string>>();
const moduleImportsSeeded = new WeakSet<Document>();
const defaultCssModuleLoader: CssModuleLoader = (specifier) =>
  import(specifier, { with: { type: 'css' } });
let loadCssModule = defaultCssModuleLoader;

/** Override CSS module loading in unit tests. */
export function setCssModuleLoaderForTests(loader?: CssModuleLoader): void {
  loadCssModule = loader ?? defaultCssModuleLoader;
}

function catalogFor(document: Document): DocumentCatalog {
  let catalog = catalogs.get(document);
  if (!catalog) {
    catalog = {
      resources: new Map(),
      closures: new Map(),
    };
    catalogs.set(document, catalog);
  }
  return catalog;
}

function isDocument(target: StyleTarget): target is Document {
  return target.nodeType === 9;
}

function owningDocument(target: StyleTarget): Document {
  return isDocument(target) ? target : target.ownerDocument;
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function styleGlobal(owner: Document): WebUIStyleGlobal | undefined {
  const configured: WebUIStyleGlobal | undefined = owner.defaultView?.__webui;
  if (configured || owner !== globalThis.document || typeof window === 'undefined') {
    return configured;
  }
  return window.__webui;
}

/** Read the owning document's configured CSP nonce for dynamic resources. */
function readNonce(target: StyleTarget = document): string {
  const owner = owningDocument(target);
  const nonce = styleGlobal(owner)?.nonce;
  if (nonce) return nonce;
  const meta = owner.querySelector<HTMLMetaElement>('meta[name="webui-nonce"]');
  return meta?.content ?? '';
}

/** Compare prepared resources without allocating serialized copies. */
export function sameComponentStyleResource(
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

/** Compare closures without allocating serialized copies. */
export function sameComponentStyleClosure(
  current: readonly string[],
  next: readonly string[],
): boolean {
  return current === next ||
    (current.length === next.length && current.every((id, index) => id === next[index]));
}

/** Validate and detach a componentStyles payload before any registry mutation. */
export function prepareComponentStyles(value: unknown): ComponentStyles | undefined {
  if (value === undefined) return undefined;
  if (!isObject(value) || value.version !== 1) {
    throw new Error('[WebUI] componentStyles must use version 1.');
  }
  if (value.strategy !== 'link' && value.strategy !== 'style' && value.strategy !== 'module') {
    throw new Error('[WebUI] componentStyles strategy must be "link", "style", or "module".');
  }
  if (!isObject(value.resources) || !isObject(value.closures)) {
    throw new Error('[WebUI] componentStyles resources and closures must be objects.');
  }

  const strategy = value.strategy;
  const resources: Record<string, ComponentStyleResource> = {};
  for (const id of Object.keys(value.resources)) {
    const resource = value.resources[id];
    if (!id || !isObject(resource) || resource.kind !== strategy) {
      throw new Error(`[WebUI] Invalid component style resource "${id}".`);
    }
    let members: string[] | undefined;
    if (resource.members !== undefined) {
      if (
        !Array.isArray(resource.members) ||
        resource.members.length < 2 ||
        resource.members.some(member => typeof member !== 'string' || !member) ||
        new Set(resource.members).size !== resource.members.length
      ) {
        throw new Error(`[WebUI] Invalid component style resource members "${id}".`);
      }
      members = [...resource.members] as string[];
    }
    if (resource.kind === 'link' && typeof resource.href === 'string' && resource.href) {
      resources[id] = members
        ? { kind: 'link', href: resource.href, members }
        : { kind: 'link', href: resource.href };
    } else if (resource.kind === 'style' && typeof resource.css === 'string') {
      resources[id] = members
        ? { kind: 'style', css: resource.css, members }
        : { kind: 'style', css: resource.css };
    } else if (
      resource.kind === 'module' &&
      typeof resource.specifier === 'string' && resource.specifier &&
      typeof resource.css === 'string'
    ) {
      resources[id] = members
        ? { kind: 'module', specifier: resource.specifier, css: resource.css, members }
        : { kind: 'module', specifier: resource.specifier, css: resource.css };
    } else {
      throw new Error(`[WebUI] Invalid component style resource "${id}".`);
    }
  }

  const closures: Record<string, string[]> = {};
  for (const root of Object.keys(value.closures)) {
    const closure = value.closures[root];
    if (!root || !Array.isArray(closure)) {
      throw new Error(`[WebUI] Invalid component style closure "${root}".`);
    }
    closures[root] = closure.map((id) => {
      if (typeof id !== 'string' || !id) {
        throw new Error(`[WebUI] Invalid resource ID in component style closure "${root}".`);
      }
      return id;
    });
  }
  return { version: 1, strategy, resources, closures };
}

/** Validate a required componentStyles payload. */
export function requireComponentStyles(value: unknown): ComponentStyles {
  const styles = prepareComponentStyles(value);
  if (!styles) {
    throw new Error('[WebUI] componentStyles is required.');
  }
  return styles;
}

/** Check a prepared payload for conflicts without publishing any definitions. */
export function validateComponentStylesRegistration(
  styles: ComponentStyles | undefined,
  document: Document = globalThis.document,
): void {
  if (!styles) return;
  const catalog = catalogFor(document);
  for (const id of Object.keys(styles.resources)) {
    const current = catalog.resources.get(id);
    if (current && !sameComponentStyleResource(current, styles.resources[id])) {
      throw new Error(`[WebUI] Conflicting component style resource "${id}".`);
    }
  }
  for (const root of Object.keys(styles.closures)) {
    const current = catalog.closures.get(root);
    const next = styles.closures[root];
    if (current && !sameComponentStyleClosure(current, next)) {
      throw new Error(`[WebUI] Conflicting component style closure "${root}".`);
    }
  }
}

/**
 * Publish a prepared payload after its conflicts have been validated.
 *
 * Internal registration paths use this to avoid cloning the payload again.
 */
export function registerPreparedComponentStyles(
  styles: ComponentStyles,
  document: Document = globalThis.document,
): void {
  const catalog = catalogFor(document);
  for (const id of Object.keys(styles.resources)) {
    if (catalog.resources.has(id)) continue;
    const resource = styles.resources[id];
    catalog.resources.set(id, resource);
  }
  for (const root of Object.keys(styles.closures)) {
    if (!catalog.closures.has(root)) {
      catalog.closures.set(root, styles.closures[root]);
    }
  }
}

/** Validate and publish definitions and ordered closures for one owning Document. */
export function registerComponentStyles(
  value: ComponentStyles | unknown,
  document: Document = globalThis.document,
): void {
  const styles = requireComponentStyles(value);
  validateComponentStylesRegistration(styles, document);
  registerPreparedComponentStyles(styles, document);
}

/** Return whether one owning Document already knows an exact resource ID. */
export function hasRegisteredComponentStyleResource(
  id: string,
  document: Document = globalThis.document,
): boolean {
  return catalogFor(document).resources.has(id);
}

function targetInstalledResources(target: StyleTarget): Set<string> {
  let targetInstalled = installed.get(target);
  if (!targetInstalled) {
    targetInstalled = new Set();
    installed.set(target, targetInstalled);
  }
  return targetInstalled;
}

function markResourceInstalled(
  targetInstalled: Set<string>,
  id: string,
  resource: ComponentStyleResource,
): void {
  targetInstalled.add(id);
  const members = resource.members;
  if (!members) return;
  for (let i = 0; i < members.length; i++) targetInstalled.add(members[i]);
}

function isMatchingResourceMarker(
  candidate: Element,
  resource: ComponentStyleResource,
): boolean {
  const strategy = candidate.getAttribute('data-webui-strategy');
  if (strategy !== null && strategy !== resource.kind) return false;
  return resource.kind === 'link'
    ? candidate.localName === 'link'
    : candidate.localName === 'style';
}

/** Whether an element is a compiler-owned Link/Style resource marker. */
export function isComponentStyleMarker(element: Element): boolean {
  if (typeof element.getAttribute !== 'function') return false;
  if (element.getAttribute('data-webui-resource') === null) return false;
  const strategy = element.getAttribute('data-webui-strategy');
  return (element.localName === 'link' && strategy === 'link') ||
    (element.localName === 'style' && (strategy === 'style' || strategy === 'module'));
}

function scanSsrMarkerElements(
  candidates: ArrayLike<Element>,
  catalog: DocumentCatalog,
  targetInstalled: Set<string>,
  state: SsrStyleMarkerState,
): void {
  for (let i = 0; i < candidates.length; i++) {
    const candidate = candidates[i];
    const id = candidate.getAttribute('data-webui-resource');
    if (id === null) continue;
    const resource = catalog.resources.get(id);
    if (!resource || !isMatchingResourceMarker(candidate, resource)) continue;
    if (resource.kind !== 'module') {
      markResourceInstalled(targetInstalled, id, resource);
      continue;
    }
    let moduleElements = state.moduleElements;
    let moduleIds = state.moduleIds;
    if (!moduleElements || !moduleIds) {
      moduleElements = new Map();
      moduleIds = [];
      state.moduleElements = moduleElements;
      state.moduleIds = moduleIds;
    }
    if (!moduleElements.has(id)) {
      moduleElements.set(id, candidate);
      moduleIds.push(id);
    }
  }
}

function ensureSsrMarkers(
  target: StyleTarget,
  catalog: DocumentCatalog,
  targetInstalled: Set<string>,
): SsrStyleMarkerState {
  let state = ssrMarkers.get(target);
  if (state) return state;
  state = {};
  ssrMarkers.set(target, state);
  const candidates = target.querySelectorAll?.(
    'link[data-webui-resource],style[data-webui-resource]',
  );
  if (candidates) {
    scanSsrMarkerElements(candidates, catalog, targetInstalled, state);
  }
  return state;
}

/**
 * Claim compiler-emitted SSR styles before installing a component closure.
 *
 * The first call scans the complete CSS tree so definition order cannot make a
 * descendant install ahead of an active route root. Streaming route markers
 * arrive later, so their generated `<webui-route>` parent is rescanned when
 * that route host activates.
 */
export function claimSsrComponentStyles(source: Element, target: StyleTarget): void {
  const catalog = catalogFor(owningDocument(target));
  const targetInstalled = targetInstalledResources(target);
  const state = ensureSsrMarkers(target, catalog, targetInstalled);
  const route = source.parentElement;
  if (route?.localName === 'webui-route') {
    scanSsrMarkerElements(route.children, catalog, targetInstalled, state);
  }
}

function markerScope(target: StyleTarget): Element | ShadowRoot {
  return isDocument(target) ? target.head : target;
}

/**
 * Anchor elements for the resources already present in a CSS tree.
 *
 * `installElements` uses these to insert new resources in closure order without
 * re-searching the tree per resource. The result is cached and kept current by
 * {@link appendResource}, because otherwise every closure install re-reads
 * `data-webui-resource` from every element in a head that we ourselves keep
 * growing — quadratic in the number of installed resources.
 */
function directResourceMarkers(
  target: StyleTarget,
  catalog: DocumentCatalog,
): Map<string, Element> | undefined {
  const scope = markerScope(target);
  const cached = resourceMarkers.get(target);
  if (cached && cached.count === scope.childElementCount) {
    return cached.markers.size > 0 ? cached.markers : undefined;
  }
  const candidates = scope.children;
  const markers = new Map<string, Element>();
  for (let i = 0; i < candidates.length; i++) {
    const id = candidates[i].getAttribute('data-webui-resource');
    if (id === null) continue;
    const resource = catalog.resources.get(id);
    if (!resource || !isMatchingResourceMarker(candidates[i], resource)) continue;
    if (!markers.has(id)) markers.set(id, candidates[i]);
  }
  resourceMarkers.set(target, { count: candidates.length, markers });
  return markers.size > 0 ? markers : undefined;
}

/** Keep the marker cache current for a resource element we just inserted. */
function noteMarkerInserted(target: StyleTarget, id: string, element: Element): void {
  const cached = resourceMarkers.get(target);
  if (!cached) return;
  // Exactly one element was inserted into the scanned scope, so the expected
  // child count advances by one without re-reading the live collection.
  cached.count++;
  if (!cached.markers.has(id)) cached.markers.set(id, element);
}

function appendResource(
  target: StyleTarget,
  id: string,
  resource: ComponentStyleResource,
  before?: Element,
): Element {
  const document = owningDocument(target);
  const parent: ParentNode = isDocument(target) ? target.head : target;
  if (resource.kind === 'link') {
    const link = document.createElement('link');
    link.rel = 'stylesheet';
    link.href = resource.href;
    link.setAttribute('data-webui-resource', id);
    link.setAttribute('data-webui-strategy', 'link');
    parent.insertBefore(link, before ?? null);
    noteMarkerInserted(target, id, link);
    return link;
  }
  if (resource.kind === 'style') {
    const style = document.createElement('style');
    const nonce = readNonce(target);
    if (nonce) style.nonce = nonce;
    style.textContent = resource.css;
    style.setAttribute('data-webui-resource', id);
    style.setAttribute('data-webui-strategy', 'style');
    parent.insertBefore(style, before ?? null);
    noteMarkerInserted(target, id, style);
    return style;
  }
  throw new Error(`[WebUI] Module resource "${id}" cannot be installed as an element.`);
}

/** Specifiers already emitted in an owning Document's SSR bootstrap payload. */
function seededModuleSpecifiers(owner: Document): Set<string> {
  let installedSpecifiers = moduleImportsInstalled.get(owner);
  if (!installedSpecifiers) {
    installedSpecifiers = new Set();
    moduleImportsInstalled.set(owner, installedSpecifiers);
  }
  if (moduleImportsSeeded.has(owner)) return installedSpecifiers;
  moduleImportsSeeded.add(owner);
  const styles = styleGlobal(owner)?.styles;
  if (!styles) return installedSpecifiers;
  for (let i = 0; i < styles.length; i++) {
    installedSpecifiers.add(styles[i]);
  }
  return installedSpecifiers;
}

/** Install one Module resource's import-map entry once per owning Document. */
function installModuleImportMap(
  target: StyleTarget,
  specifier: string,
  css: string,
): void {
  const owner = owningDocument(target);
  const installedSpecifiers = seededModuleSpecifiers(owner);
  if (installedSpecifiers.has(specifier)) return;

  const script = owner.createElement('script');
  script.type = 'importmap';
  const nonce = readNonce(target);
  if (nonce) script.nonce = nonce;
  script.textContent = JSON.stringify({
    imports: { [specifier]: `data:text/css,${encodeURIComponent(css)}` },
  });
  owner.head.appendChild(script);
  installedSpecifiers.add(specifier);
}

function loadModule(
  target: StyleTarget,
  id: string,
  resource: { specifier: string; css: string },
): Promise<CSSStyleSheet> {
  let targetPending = pending.get(target);
  if (!targetPending) {
    targetPending = new Map();
    pending.set(target, targetPending);
  }
  const existing = targetPending.get(id);
  if (existing) return existing;

  installModuleImportMap(target, resource.specifier, resource.css);

  const loads = targetPending;
  const promise = loadCssModule(resource.specifier)
    .then((module: CssModule) => module.default);
  loads.set(id, promise);
  // Keep successful loads for another caller to adopt; only failures retry.
  void promise.catch(() => {
    if (loads.get(id) === promise) loads.delete(id);
  });
  return promise;
}

function markClosureComplete(target: StyleTarget, rootId: string): void {
  let roots = completedClosures.get(target);
  if (!roots) {
    roots = new Set();
    completedClosures.set(target, roots);
  }
  roots.add(rootId);
}

function installElementRange(
  target: StyleTarget,
  catalog: DocumentCatalog,
  closure: readonly string[],
  targetInstalled: Set<string>,
  start: number,
  end: number,
  before?: Element,
  linkElements?: HTMLLinkElement[],
): void {
  for (let i = start; i < end; i++) {
    const id = closure[i];
    if (targetInstalled.has(id)) continue;
    const resource = catalog.resources.get(id);
    if (!resource || resource.kind === 'module') continue;
    const element = appendResource(target, id, resource, before);
    if (linkElements && resource.kind === 'link') {
      linkElements.push(element as HTMLLinkElement);
    }
    markResourceInstalled(targetInstalled, id, resource);
  }
}

/**
 * Warn when a closure asks for an order the tree has already contradicted.
 *
 * Resources are installed once and shared, so the first closure to run fixes
 * their relative order. A later closure that wants the opposite order gets
 * silently ignored — its rules cascade the wrong way with no signal. Detect it
 * by checking that every anchor we found still appears in closure order.
 */
function warnOnContradictoryOrder(
  rootId: string,
  closure: readonly string[],
  markers: Map<string, Element>,
): void {
  let previousId: string | undefined;
  let previous: Element | undefined;
  for (let i = 0; i < closure.length; i++) {
    const marker = markers.get(closure[i]);
    if (!marker) continue;
    if (
      previous &&
      typeof previous.compareDocumentPosition === 'function' &&
      // Bit 4 is DOCUMENT_POSITION_FOLLOWING: `marker` comes after `previous`,
      // which is what closure order requires.
      (previous.compareDocumentPosition(marker) & 4) === 0
    ) {
      console.warn(
        `[WebUI] Closure "${rootId}" wants "${previousId}" before "${closure[i]}", but they are already installed in the opposite order. The first closure to install decides, so these rules cascade against this closure's intent.`,
      );
      return;
    }
    previousId = closure[i];
    previous = marker;
  }
}

function installElements(
  target: StyleTarget,
  catalog: DocumentCatalog,
  closure: readonly string[],
  targetInstalled: Set<string>,
  markers: Map<string, Element> | undefined,
  rootId: string,
  linkElements?: HTMLLinkElement[],
): void {
  // Existing markers delimit ranges that can be inserted in closure order
  // without searching the target again for every resource.
  let segmentEnd = closure.length;
  let before: Element | undefined;
  if (markers) {
    if (DEV) warnOnContradictoryOrder(rootId, closure, markers);
    for (let i = closure.length - 1; i >= 0; i--) {
      const marker = markers.get(closure[i]);
      if (!marker) continue;
      installElementRange(
        target,
        catalog,
        closure,
        targetInstalled,
        i + 1,
        segmentEnd,
        before,
        linkElements,
      );
      const resource = catalog.resources.get(closure[i]);
      if (resource && resource.kind !== 'module') {
        markResourceInstalled(targetInstalled, closure[i], resource);
      }
      before = marker;
      segmentEnd = i;
    }
  }
  installElementRange(
    target,
    catalog,
    closure,
    targetInstalled,
    0,
    segmentEnd,
    before,
    linkElements,
  );
}

function moduleAdoptionState(target: StyleTarget): ModuleAdoptionState {
  let state = moduleAdoption.get(target);
  if (!state) {
    state = { order: [], reserved: new Set(), sheets: new Map() };
    moduleAdoption.set(target, state);
  }
  return state;
}

/** Reserve a stored-order slot before the module load starts. */
function reserveModuleSlot(target: StyleTarget, id: string): void {
  const state = moduleAdoptionState(target);
  if (state.reserved.has(id)) return;
  state.reserved.add(id);
  state.order.push(id);
}

/**
 * Rewrite `adoptedStyleSheets` so framework sheets follow reserved order.
 *
 * Sheets adopted by application code keep their relative order ahead of the
 * framework's, matching the append-only behavior of a single closure install.
 * The array is only assigned when it actually changes, since assignment
 * invalidates style for the whole tree.
 */
function applyModuleOrder(target: StyleTarget, state: ModuleAdoptionState): void {
  const adopted = target.adoptedStyleSheets;
  const owned = new Set(state.sheets.values());
  const next: CSSStyleSheet[] = [];
  for (let i = 0; i < adopted.length; i++) {
    if (!owned.has(adopted[i])) next.push(adopted[i]);
  }
  const seen = new Set<CSSStyleSheet>();
  for (let i = 0; i < state.order.length; i++) {
    const sheet = state.sheets.get(state.order[i]);
    if (!sheet || seen.has(sheet)) continue;
    seen.add(sheet);
    next.push(sheet);
  }
  if (next.length === adopted.length) {
    let same = true;
    for (let i = 0; i < next.length; i++) {
      if (next[i] !== adopted[i]) {
        same = false;
        break;
      }
    }
    if (same) return;
  }
  target.adoptedStyleSheets = next;
}

function commitModules(
  target: StyleTarget,
  ids: readonly string[],
  sheets: readonly CSSStyleSheet[],
  markers: Map<string, Element> | undefined,
  targetInstalled: Set<string>,
): void {
  const state = moduleAdoptionState(target);
  const catalog = catalogFor(owningDocument(target));
  for (let i = 0; i < sheets.length; i++) {
    const id = ids[i];
    if (!state.reserved.has(id)) {
      state.reserved.add(id);
      state.order.push(id);
    }
    state.sheets.set(id, sheets[i]);
  }
  applyModuleOrder(target, state);
  const targetPending = pending.get(target);
  for (let i = 0; i < sheets.length; i++) {
    const id = ids[i];
    const resource = catalog.resources.get(id);
    if (resource) markResourceInstalled(targetInstalled, id, resource);
    else targetInstalled.add(id);
    targetPending?.delete(id);
    // Drop the element reference with the node so an adopted fallback does not
    // stay reachable for the lifetime of the CSS tree.
    const marker = markers?.get(id);
    if (marker) {
      marker.remove();
      markers?.delete(id);
    }
  }
  if (targetPending?.size === 0) pending.delete(target);
}

async function finishModuleLoads(
  target: StyleTarget,
  rootId: string,
  ids: readonly string[],
  loads: readonly Promise<CSSStyleSheet>[],
  markers: Map<string, Element> | undefined,
  targetInstalled: Set<string>,
): Promise<void> {
  const sheets: CSSStyleSheet[] = [];
  try {
    // Loads are already in flight; awaiting them in closure order preserves
    // the successful prefix when a later resource rejects.
    for (let i = 0; i < loads.length; i++) {
      sheets.push(await loads[i]);
    }
  } catch (error) {
    if (sheets.length > 0) {
      commitModules(target, ids, sheets, markers, targetInstalled);
    }
    throw error;
  }
  commitModules(target, ids, sheets, markers, targetInstalled);
  markClosureComplete(target, rootId);
}

/**
 * Install one root's ordered style closure into a Document or ShadowRoot.
 *
 * Link and Style resources complete synchronously and return undefined. Module
 * resources return a promise that rejects on import failure; failed pending
 * entries are removed for retry.
 *
 * Failure policy — the two outcomes are deliberately different:
 *
 * - **An unknown `rootId` returns quietly.** A component with no CSS has no
 *   closure at all, and every mounting host calls this unconditionally, so a
 *   missing closure is the normal "nothing to install" answer, not an error.
 * - **A known closure naming an unknown resource throws.** The compiler emits
 *   closures and resources together, so a closure that references an ID the
 *   catalog does not define means the payload was truncated or mismatched.
 *   Rendering on would leave the component visibly unstyled with no signal.
 */
export function installComponentStyles(
  rootId: string,
  target: StyleTarget,
  host?: HTMLElement,
): Promise<void> | undefined {
  const catalog = catalogFor(owningDocument(target));
  const closure = catalog.closures.get(rootId);
  if (!closure || completedClosures.get(target)?.has(rootId)) return undefined;

  const targetInstalled = targetInstalledResources(target);
  const markerState = ensureSsrMarkers(target, catalog, targetInstalled);

  let hasElementWork = false;
  let hasClosureModuleWork = false;
  for (let i = 0; i < closure.length; i++) {
    const id = closure[i];
    if (targetInstalled.has(id)) continue;
    const resource = catalog.resources.get(id);
    if (!resource) {
      throw new Error(`[WebUI] Missing component style resource "${id}" for closure "${rootId}".`);
    }
    if (resource.kind === 'module') hasClosureModuleWork = true;
    else hasElementWork = true;
  }

  let hasSsrModuleWork = false;
  const ssrModuleIds = markerState.moduleIds;
  if (ssrModuleIds) {
    for (let i = 0; i < ssrModuleIds.length; i++) {
      if (!targetInstalled.has(ssrModuleIds[i])) {
        hasSsrModuleWork = true;
        break;
      }
    }
  }
  if (!hasElementWork && !hasClosureModuleWork && !hasSsrModuleWork) {
    markClosureComplete(target, rootId);
    return undefined;
  }

  if (hasElementWork) {
    const linkElements: HTMLLinkElement[] = [];
    installElements(
      target,
      catalog,
      closure,
      targetInstalled,
      directResourceMarkers(target, catalog),
      rootId,
      linkElements,
    );
    if (host && !isDocument(target)) {
      installComponentLinkStyles(host, target, linkElements);
    }
  }

  let moduleIds: string[] | undefined;
  let moduleLoads: Promise<CSSStyleSheet>[] | undefined;
  let queuedModules: Set<string> | undefined;
  const queueModule = (id: string, resource: { specifier: string; css: string }): void => {
    if (!moduleIds || !moduleLoads || !queuedModules) {
      moduleIds = [];
      moduleLoads = [];
      queuedModules = new Set();
    }
    reserveModuleSlot(target, id);
    moduleIds.push(id);
    moduleLoads.push(loadModule(target, id, resource));
    queuedModules.add(id);
  };
  if (ssrModuleIds) {
    for (let i = 0; i < ssrModuleIds.length; i++) {
      const id = ssrModuleIds[i];
      if (targetInstalled.has(id)) continue;
      const resource = catalog.resources.get(id);
      if (!resource || resource.kind !== 'module') continue;
      queueModule(id, resource);
    }
  }
  for (let i = 0; i < closure.length; i++) {
    const id = closure[i];
    if (targetInstalled.has(id) || queuedModules?.has(id)) continue;
    const resource = catalog.resources.get(id);
    if (!resource) {
      throw new Error(`[WebUI] Missing component style resource "${id}" for closure "${rootId}".`);
    }
    if (resource.kind === 'module') {
      queueModule(id, resource);
    }
  }
  if (!moduleLoads || !moduleIds) {
    markClosureComplete(target, rootId);
    return undefined;
  }
  return finishModuleLoads(
    target,
    rootId,
    moduleIds,
    moduleLoads,
    markerState.moduleElements,
    targetInstalled,
  );
}
