// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/** One addressable component stylesheet definition. */
export type ComponentStyleResource =
  | { kind: 'link'; href: string }
  | { kind: 'style'; css: string }
  | { kind: 'module'; specifier: string; css: string };

/** Versioned stylesheet catalog shared by server and client registration paths. */
export interface ComponentStyles {
  version: 1;
  strategy: 'link' | 'style' | 'module';
  resources: Record<string, ComponentStyleResource>;
  closures: Record<string, string[]>;
}

interface DocumentCatalog {
  readonly resources: Map<string, ComponentStyleResource>;
  readonly resourceKeys: Map<string, string>;
  readonly closures: Map<string, readonly string[]>;
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
const pending = new WeakMap<StyleTarget, Map<string, Promise<void>>>();
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
      resourceKeys: new Map(),
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
export function readNonce(target: StyleTarget = document): string {
  const owner = owningDocument(target);
  const nonce = styleGlobal(owner)?.nonce;
  if (nonce) return nonce;
  const meta = owner.querySelector<HTMLMetaElement>('meta[name="webui-nonce"]');
  return meta?.content ?? '';
}

function resourceKey(resource: ComponentStyleResource): string {
  switch (resource.kind) {
    case 'link':
      return `link\0${resource.href}`;
    case 'style':
      return `style\0${resource.css}`;
    case 'module':
      return `module\0${resource.specifier}\0${resource.css}`;
  }
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
    if (resource.kind === 'link' && typeof resource.href === 'string' && resource.href) {
      resources[id] = { kind: 'link', href: resource.href };
    } else if (resource.kind === 'style' && typeof resource.css === 'string') {
      resources[id] = { kind: 'style', css: resource.css };
    } else if (
      resource.kind === 'module' &&
      typeof resource.specifier === 'string' && resource.specifier &&
      typeof resource.css === 'string'
    ) {
      resources[id] = { kind: 'module', specifier: resource.specifier, css: resource.css };
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
    const nextKey = resourceKey(styles.resources[id]);
    const currentKey = catalog.resourceKeys.get(id);
    if (currentKey !== undefined && currentKey !== nextKey) {
      throw new Error(`[WebUI] Conflicting component style resource "${id}".`);
    }
  }
  for (const root of Object.keys(styles.closures)) {
    const current = catalog.closures.get(root);
    const next = styles.closures[root];
    if (current && (
      current.length !== next.length ||
      current.some((id, index) => id !== next[index])
    )) {
      throw new Error(`[WebUI] Conflicting component style closure "${root}".`);
    }
  }
}

/** Publish validated definitions and ordered closures for one owning Document. */
export function registerComponentStyles(
  value: ComponentStyles | unknown,
  document: Document = globalThis.document,
): void {
  const styles = requireComponentStyles(value);
  validateComponentStylesRegistration(styles, document);
  const catalog = catalogFor(document);
  for (const id of Object.keys(styles.resources)) {
    if (catalog.resources.has(id)) continue;
    const resource = styles.resources[id];
    catalog.resources.set(id, resource);
    catalog.resourceKeys.set(id, resourceKey(resource));
  }
  for (const root of Object.keys(styles.closures)) {
    if (!catalog.closures.has(root)) {
      catalog.closures.set(root, styles.closures[root]);
    }
  }
}

/** Return whether one owning Document already knows an exact resource ID. */
export function hasRegisteredComponentStyleResource(
  id: string,
  document: Document = globalThis.document,
): boolean {
  return catalogFor(document).resources.has(id);
}

/** Return whether one owning Document knows an ordered closure for a root. */
export function hasRegisteredComponentStyleClosure(
  rootId: string,
  document: Document = globalThis.document,
): boolean {
  return catalogFor(document).closures.has(rootId);
}

function markerFor(target: StyleTarget, id: string): Element | undefined {
  const scope: Element | ShadowRoot = isDocument(target) ? target.head : target;
  const candidates = scope.children;
  for (let i = 0; i < candidates.length; i++) {
    if (candidates[i].getAttribute('data-webui-resource') === id) {
      return candidates[i];
    }
  }
  return undefined;
}

function appendResource(
  target: StyleTarget,
  id: string,
  resource: ComponentStyleResource,
  before?: Element,
): void {
  const document = owningDocument(target);
  const parent: ParentNode = isDocument(target) ? target.head : target;
  if (resource.kind === 'link') {
    const link = document.createElement('link');
    link.rel = 'stylesheet';
    link.href = resource.href;
    link.setAttribute('data-webui-resource', id);
    parent.insertBefore(link, before ?? null);
    return;
  }
  if (resource.kind === 'style') {
    const style = document.createElement('style');
    const nonce = readNonce(target);
    if (nonce) style.nonce = nonce;
    style.textContent = resource.css;
    style.setAttribute('data-webui-resource', id);
    parent.insertBefore(style, before ?? null);
    return;
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

async function installModule(
  target: StyleTarget,
  id: string,
  resource: { specifier: string; css: string },
): Promise<void> {
  let targetPending = pending.get(target);
  if (!targetPending) {
    targetPending = new Map();
    pending.set(target, targetPending);
  }
  const existing = targetPending.get(id);
  if (existing) return existing;

  installModuleImportMap(target, resource.specifier, resource.css);

  const targetInstalled = installed.get(target) ?? new Set<string>();
  installed.set(target, targetInstalled);
  const promise = loadCssModule(resource.specifier)
    .then((module: CssModule) => {
      if (!target.adoptedStyleSheets.includes(module.default)) {
        target.adoptedStyleSheets = [...target.adoptedStyleSheets, module.default];
      }
      targetInstalled.add(id);
      markerFor(target, id)?.remove();
    })
    .finally(() => {
      targetPending!.delete(id);
    });
  targetPending.set(id, promise);
  return promise;
}

/**
 * Install one root's ordered style closure into a Document or ShadowRoot.
 *
 * Link and Style resources are synchronous. Module resources return a promise
 * that rejects on import failure; failed pending entries are removed for retry.
 */
export function installComponentStyles(
  rootId: string,
  target: StyleTarget,
): Promise<void> {
  const catalog = catalogFor(owningDocument(target));
  const closure = catalog.closures.get(rootId);
  if (!closure) return Promise.resolve();

  let targetInstalled = installed.get(target);
  if (!targetInstalled) {
    targetInstalled = new Set();
    installed.set(target, targetInstalled);
  }
  let moduleInstalls = Promise.resolve();
  for (let i = 0; i < closure.length; i++) {
    const id = closure[i];
    if (targetInstalled.has(id)) continue;
    const resource = catalog.resources.get(id);
    if (!resource) {
      throw new Error(`[WebUI] Missing component style resource "${id}" for closure "${rootId}".`);
    }
    if (resource.kind === 'module') {
      moduleInstalls = moduleInstalls.then(
        () => installModule(target, id, resource),
      );
      continue;
    }
    if (!markerFor(target, id)) {
      let before: Element | undefined;
      for (let j = i + 1; j < closure.length && !before; j++) {
        before = markerFor(target, closure[j]);
      }
      appendResource(target, id, resource, before);
    }
    targetInstalled.add(id);
  }
  return moduleInstalls;
}
