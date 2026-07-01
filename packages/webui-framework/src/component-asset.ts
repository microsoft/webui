// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Static component asset loader for the WebUI Framework plugin.
 *
 * Kept outside the framework root entrypoint so apps that only hydrate normal
 * WebUI components do not load this optional CDN/static-asset helper.
 */

import {
  getTemplate,
  registerTemplateData,
  type CompiledConditionFn,
  type TemplateMeta,
} from './template.js';

const ASSET_TYPE = 'webui-component-asset';
const ASSET_VERSION = 1;

const injectedAssetStyles = new Set<string>();
const assetLoadPromises = new Map<string, Promise<void>>();
let assetStylesSeeded = false;

/** Static WebUI Framework component asset emitted by `webui build --emit-component-assets`. */
export interface ComponentAsset {
  type?: 'webui-component-asset';
  version?: number;
  components?: string[];
  templateStyles?: string[];
  templates?: Record<string, TemplateMeta>;
  templateFunctions?: Record<string, CompiledConditionFn[]>;
}

/** Options for loading or registering a static component asset. */
export interface ComponentAssetOptions {
  /** CSP nonce for importmap style scripts. Defaults to WebUI SSR metadata. */
  nonce?: string;
}

/** State payload returned by a lazy component data loader. */
export type ComponentAssetState = Record<string, unknown>;

/** Manifest entry for one lazy component root. */
export interface ComponentAssetManifestEntry<Data extends ComponentAssetState = ComponentAssetState> {
  /** Static component asset module emitted by `webui build --emit-component-assets`. */
  asset: string | URL;
  /** JavaScript module that defines/registers the custom element class. */
  module?: () => Promise<unknown>;
  /** Optional data request kicked off in parallel with asset/module loading. */
  data?: () => Promise<Data>;
}

/** Map of component tag name to lazy asset metadata. */
export type ComponentAssetManifest = Record<string, ComponentAssetManifestEntry>;

/** In-flight or completed work for one lazy component root. */
export interface ComponentAssetPreload<Data extends ComponentAssetState = ComponentAssetState> {
  /** Static WebUI template/style asset registration. */
  asset: Promise<void>;
  /** Optional JavaScript module import. */
  module?: Promise<unknown>;
  /** Optional data request. */
  data?: Promise<Data>;
}

/** Options for creating a lazy component element from a manifest entry. */
export interface ComponentAssetCreateOptions {
  /** Wait for data before returning the element. Defaults to false. */
  awaitData?: boolean;
  /** Maximum time to wait for data when awaitData is true. */
  dataTimeoutMs?: number;
}

/** Loader returned by `defineComponentAssets`. */
export interface ComponentAssetRegistry {
  /** Start asset, module, and optional data work for a component. */
  preload<Data extends ComponentAssetState = ComponentAssetState>(tag: string): ComponentAssetPreload<Data>;
  /** Create a component element and apply loaded data via setState(), if present. */
  create<Data extends ComponentAssetState = ComponentAssetState>(
    tag: string,
    options?: ComponentAssetCreateOptions,
  ): Promise<HTMLElement>;
}

interface WebUIAssetGlobal {
  nonce?: string;
  styles?: string[];
  [key: string]: unknown;
}

function assetGlobal(): WebUIAssetGlobal | undefined {
  return window.__webui as WebUIAssetGlobal | undefined;
}

/** Define a reusable manifest-driven loader for static component assets. */
export function defineComponentAssets(manifest: ComponentAssetManifest): ComponentAssetRegistry {
  const preloads = new Map<string, ComponentAssetPreload>();

  function preload<Data extends ComponentAssetState = ComponentAssetState>(tag: string): ComponentAssetPreload<Data> {
    const existing = preloads.get(tag) as ComponentAssetPreload<Data> | undefined;
    if (existing) return existing;

    const entry = manifest[tag];
    if (!entry) {
      throw new Error(`[WebUI] No component asset manifest entry for <${tag}>.`);
    }

    const next: ComponentAssetPreload<Data> = {
      asset: loadComponentAsset(tag, entry.asset),
    };
    if (entry.module) {
      next.module = entry.module();
    }
    if (entry.data) {
      next.data = entry.data() as Promise<Data>;
    }
    next.asset.catch(() => {});
    next.module?.catch(() => {});
    next.data?.catch(() => {});
    preloads.set(tag, next);
    return next;
  }

  async function create<Data extends ComponentAssetState = ComponentAssetState>(
    tag: string,
    options: ComponentAssetCreateOptions = {},
  ): Promise<HTMLElement> {
    const pending = preload<Data>(tag);
    await waitForElementResources(pending);
    const element = document.createElement(tag);
    if (pending.data) {
      if (options.awaitData) {
        const state = options.dataTimeoutMs === undefined
          ? await pending.data
          : await dataWithTimeout(pending.data, options.dataTimeoutMs);
        if (state) {
          applyState(element, state);
        } else {
          applyDataWhenReady(element, pending.data);
        }
      } else {
        applyDataWhenReady(element, pending.data);
      }
    }
    return element;
  }

  return { preload, create };
}

async function waitForElementResources(pending: ComponentAssetPreload): Promise<void> {
  await pending.asset;
  if (pending.module) await pending.module;
}

function applyState(element: HTMLElement, state: ComponentAssetState): void {
  const setState = (element as unknown as { setState?: (state: ComponentAssetState) => void }).setState;
  if (typeof setState === 'function') {
    setState.call(element, state);
  }
}

function applyDataWhenReady(element: HTMLElement, data: Promise<ComponentAssetState>): void {
  const elementRef = new WeakRef(element);
  void data.then(state => {
    const liveElement = elementRef.deref();
    if (liveElement) applyState(liveElement, state);
  }).catch(() => {});
}

function dataWithTimeout<Data extends ComponentAssetState>(
  data: Promise<Data>,
  timeoutMs: number,
): Promise<Data | undefined> {
  if (timeoutMs < 0) return data;
  return Promise.race([
    data,
    new Promise<undefined>(resolve => {
      setTimeout(() => resolve(undefined), timeoutMs);
    }),
  ]);
}

/** Import and register a static component asset emitted by the WebUI CLI. */
function loadComponentAsset(
  tag: string,
  url: string | URL,
  options: ComponentAssetOptions = {},
): Promise<void> {
  if (getTemplate(tag)) return Promise.resolve();

  const assetUrl = new URL(url, document.baseURI);
  const href = assetUrl.href;
  let promise = assetLoadPromises.get(href);
  if (promise) return promise;

  promise = importAndRegisterComponentAsset(assetUrl, options)
    .finally(() => {
      assetLoadPromises.delete(href);
    });
  assetLoadPromises.set(href, promise);
  return promise;
}

/** Register a static component asset object that has already been imported. */
function registerComponentAsset(
  asset: ComponentAsset,
  options: ComponentAssetOptions = {},
): void {
  validateAsset(asset);
  if (asset.templates && templatesAlreadyRegistered(asset.templates)) return;

  registerAssetStyles(asset.templateStyles, options.nonce ?? readNonce());

  if (asset.templates) {
    registerTemplateData(asset.templates, asset.templateFunctions);
  }
}

async function importAndRegisterComponentAsset(
  assetUrl: URL,
  options: ComponentAssetOptions,
): Promise<void> {
  const imported: unknown = await import(assetUrl.href);
  registerComponentAsset(readComponentAssetModule(imported), options);
}

function readComponentAssetModule(module: unknown): ComponentAsset {
  if (!isObject(module) || !isObject(module.default)) {
    throw new Error('[WebUI] Component asset module must default-export an asset object.');
  }
  return module.default as ComponentAsset;
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function validateAsset(asset: ComponentAsset): void {
  if (asset.type !== ASSET_TYPE) {
    throw new Error(`[WebUI] Invalid component asset type: ${String(asset.type)}`);
  }
  if (asset.version !== ASSET_VERSION) {
    throw new Error(`[WebUI] Unsupported component asset version: ${String(asset.version)}`);
  }
}

function templatesAlreadyRegistered(templates: Record<string, TemplateMeta>): boolean {
  const tags = Object.keys(templates);
  if (tags.length === 0) return false;
  for (let i = 0; i < tags.length; i++) {
    if (!getTemplate(tags[i])) return false;
  }
  return true;
}

function readNonce(): string {
  const nonce = assetGlobal()?.nonce;
  if (nonce) return nonce;
  const meta = document.querySelector('meta[name="webui-nonce"]') as HTMLMetaElement | null;
  return meta?.content ?? '';
}

function seedAssetStyleSet(): void {
  if (assetStylesSeeded) return;
  assetStylesSeeded = true;
  const styles = assetGlobal()?.styles;
  if (!styles) return;
  for (let i = 0; i < styles.length; i++) {
    injectedAssetStyles.add(styles[i]);
  }
}

function registerAssetStyles(templateStyles: string[] | undefined, nonce: string): void {
  if (!templateStyles || templateStyles.length === 0) return;
  seedAssetStyleSet();

  for (let i = 0; i < templateStyles.length; i++) {
    const imports = parseImportMap(templateStyles[i]);
    const nextImports: Record<string, string> = {};
    let hasNewImport = false;
    const specifiers = Object.keys(imports);
    for (let j = 0; j < specifiers.length; j++) {
      const specifier = specifiers[j];
      if (injectedAssetStyles.has(specifier)) continue;
      injectedAssetStyles.add(specifier);
      nextImports[specifier] = imports[specifier];
      hasNewImport = true;
    }
    if (!hasNewImport) continue;

    const script = document.createElement('script');
    script.type = 'importmap';
    if (nonce) script.nonce = nonce;
    script.textContent = JSON.stringify({ imports: nextImports });
    document.head.appendChild(script);
  }
}

function parseImportMap(scriptMarkup: string): Record<string, string> {
  const trimmed = scriptMarkup.trim();
  if (!trimmed.startsWith('<script')) {
    throw new Error('[WebUI] Component asset templateStyles entry must be a <script type="importmap"> tag.');
  }
  const openTagEnd = trimmed.indexOf('>');
  const closeTagStart = trimmed.lastIndexOf('</script>');
  if (openTagEnd < 0 || closeTagStart <= openTagEnd) {
    throw new Error('[WebUI] Component asset importmap tag is malformed.');
  }

  const parsed = JSON.parse(trimmed.substring(openTagEnd + 1, closeTagStart)) as {
    imports?: Record<string, unknown>;
  };
  if (!parsed.imports || typeof parsed.imports !== 'object') {
    throw new Error('[WebUI] Component asset importmap is missing an imports object.');
  }

  const imports: Record<string, string> = {};
  const specifiers = Object.keys(parsed.imports);
  for (let i = 0; i < specifiers.length; i++) {
    const specifier = specifiers[i];
    const uri = parsed.imports[specifier];
    if (typeof uri !== 'string' || !uri.startsWith('data:text/css,')) {
      throw new Error(`[WebUI] Component asset importmap entry "${specifier}" must be a data:text/css URI.`);
    }
    imports[specifier] = uri;
  }
  return imports;
}
