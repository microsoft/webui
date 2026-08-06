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
  hasRegisteredComponentStyleResource,
  registerComponentStyles,
  requireComponentStyles,
  validateComponentStylesRegistration,
  type ComponentStyles,
} from '../element/styles.js';

const assetModulePromises = new Map<string, Promise<unknown>>();

interface PreparedComponentAsset {
  asset: ComponentAsset;
  componentStyles: ComponentStyles;
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
  const graph = [...chunks, root];
  validatePreparedGraph(graph);
  for (let i = 0; i < chunks.length; i++) {
    registerComponentResources(chunks[i]);
  }
  registerComponentResources(root);
  for (let i = 0; i < chunks.length; i++) {
    registerComponentTemplates(chunks[i]);
  }
  registerComponentTemplates(root);
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
  const componentStyles = requireComponentStyles(asset.componentStyles);
  prepareAssetTemplateData(asset.templates, asset.templateFunctions);
  return {
    asset,
    componentStyles,
  };
}

function registerComponentResources(prepared: PreparedComponentAsset): void {
  registerComponentStyles(prepared.componentStyles);
}

function registerComponentTemplates(prepared: PreparedComponentAsset): void {
  const { asset } = prepared;
  if (componentsAlreadyRegistered(asset.components)) return;
  registerTemplateData(asset.templates, asset.templateFunctions);
}

function validatePreparedGraph(graph: readonly PreparedComponentAsset[]): void {
  const resourceKeys = new Map<string, string>();
  const closures = new Map<string, string>();
  const provided = new Set<string>();
  for (let i = 0; i < graph.length; i++) {
    const prepared = graph[i];
    validateComponentStylesRegistration(prepared.componentStyles);
    for (const component of prepared.asset.components) provided.add(component);
    const styles = prepared.componentStyles;
    for (const id of Object.keys(styles.resources)) {
      const key = JSON.stringify(styles.resources[id]);
      const current = resourceKeys.get(id);
      if (current !== undefined && current !== key) {
        throw new Error(`[WebUI] Conflicting component style resource "${id}".`);
      }
      resourceKeys.set(id, key);
    }
    for (const root of Object.keys(styles.closures)) {
      const key = JSON.stringify(styles.closures[root]);
      const current = closures.get(root);
      if (current !== undefined && current !== key) {
        throw new Error(`[WebUI] Conflicting component style closure "${root}".`);
      }
      closures.set(root, key);
    }
  }
  for (let i = 0; i < graph.length; i++) {
    const styles = graph[i].componentStyles;
    for (const root of Object.keys(styles.closures)) {
      for (const id of styles.closures[root]) {
        if (
          !resourceKeys.has(id) &&
          !hasRegisteredComponentStyleResource(id)
        ) {
          throw new Error(
            `[WebUI] Component style closure "${root}" references missing resource "${id}".`,
          );
        }
      }
    }
  }
  for (let i = 0; i < graph.length; i++) {
    const asset = graph[i].asset;
    for (const required of asset.requiredComponents) {
      if (!provided.has(required) && !getTemplate(required)) {
        throw new Error(
          `[WebUI] Component asset is missing required template: <${required}>.`,
        );
      }
    }
  }
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
