// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  getTemplate,
  prepareAssetTemplateData,
  registerTemplateData,
} from '../template.js';
import {
  readComponentAssetModule,
  sameComponents,
  validateAsset,
  type ComponentAsset,
  type ComponentAssetImport,
} from './asset.js';
import {
  prepareAssetStyles,
  readNonce,
  registerAssetStyles,
  type PreparedAssetStyles,
} from './resources.js';

const assetModulePromises = new Map<string, Promise<unknown>>();

interface PreparedComponentAsset {
  asset: ComponentAsset;
  styles: PreparedAssetStyles;
}

/** Import, validate, and atomically register one component asset graph. */
export function loadComponentAsset(
  tag: string,
  url: string | URL,
): Promise<void> {
  const assetUrl = new URL(url, document.baseURI);
  const href = assetUrl.href;
  return loadAssetModule(href, () => import(assetUrl.href))
    .then(imported => registerRootAsset(tag, href, imported));
}

async function registerRootAsset(
  expectedRoot: string,
  href: string,
  imported: unknown,
): Promise<void> {
  const asset = readComponentAssetModule(imported);
  validateAsset(asset, 'root');
  if (asset.root !== expectedRoot) {
    throw new Error(
      `[WebUI] Component asset manifest expected <${expectedRoot}> but ${href} exports <${String(asset.root)}>.`,
    );
  }
  validateExternalComponents(asset);

  const root = prepareComponentPayload(asset);
  const chunks = await prepareAssetImports(asset.imports);
  for (let i = 0; i < chunks.length; i++) {
    registerComponentPayload(chunks[i]);
  }
  registerComponentPayload(root);
}

function loadAssetModule(
  href: string,
  load: () => Promise<unknown>,
): Promise<unknown> {
  let promise = assetModulePromises.get(href);
  if (promise) return promise;

  promise = Promise.resolve()
    .then(load)
    .finally(() => {
      assetModulePromises.delete(href);
    });
  assetModulePromises.set(href, promise);
  return promise;
}

async function prepareAssetImports(
  imports: ComponentAssetImport[],
): Promise<PreparedComponentAsset[]> {
  const pending: Promise<PreparedComponentAsset>[] = [];
  for (let i = 0; i < imports.length; i++) {
    const assetImport = imports[i];
    if (componentsAlreadyRegistered(assetImport.components)) continue;
    pending.push(importAndPrepareChunk(assetImport));
  }
  return Promise.all(pending);
}

function importAndPrepareChunk(
  assetImport: ComponentAssetImport,
): Promise<PreparedComponentAsset> {
  const href = new URL(assetImport.href, document.baseURI).href;
  return loadAssetModule(href, assetImport.load)
    .then(imported => {
      const chunk = readComponentAssetModule(imported);
      validateAsset(chunk, 'chunk');
      if (!sameComponents(chunk.components, assetImport.components)) {
        throw new Error(
          `[WebUI] Shared component asset ${href} does not provide the components declared by its root import.`,
        );
      }
      return prepareComponentPayload(chunk);
    });
}

function prepareComponentPayload(asset: ComponentAsset): PreparedComponentAsset {
  prepareAssetTemplateData(asset.templates, asset.templateFunctions);
  return {
    asset,
    styles: prepareAssetStyles(asset.templateStyles),
  };
}

function registerComponentPayload(prepared: PreparedComponentAsset): void {
  const { asset, styles } = prepared;
  if (componentsAlreadyRegistered(asset.components)) return;

  registerAssetStyles(styles, readNonce());
  registerTemplateData(asset.templates, asset.templateFunctions);
  validateRequiredComponents(asset);
}

function componentsAlreadyRegistered(components: readonly string[]): boolean {
  if (components.length === 0) return false;
  for (let i = 0; i < components.length; i++) {
    if (!getTemplate(components[i])) return false;
  }
  return true;
}

function validateExternalComponents(asset: ComponentAsset): void {
  const missing: string[] = [];
  for (let i = 0; i < asset.externalComponents.length; i++) {
    const component = asset.externalComponents[i];
    if (!getTemplate(component)) missing.push(component);
  }
  if (missing.length === 0) return;

  throw new Error(
    `[WebUI] Component asset requires entr${missing.length === 1 ? 'y template' : 'y templates'} ${missing.map(tag => `<${tag}>`).join(', ')}. Load the application entry bundle and protocol before deferred component assets.`,
  );
}

function validateRequiredComponents(asset: ComponentAsset): void {
  const missing: string[] = [];
  for (let i = 0; i < asset.requiredComponents.length; i++) {
    const component = asset.requiredComponents[i];
    if (!getTemplate(component)) missing.push(component);
  }
  if (missing.length === 0) return;

  throw new Error(
    `[WebUI] Component asset is missing required template${missing.length === 1 ? '' : 's'}: ${missing.map(tag => `<${tag}>`).join(', ')}. Ensure the application entry bundle and protocol are loaded before deferred component assets.`,
  );
}
