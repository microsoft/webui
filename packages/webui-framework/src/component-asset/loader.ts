// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  prepareAssetComponentStyles,
  readComponentAssetModule,
  validateAsset,
  type ComponentAsset,
} from './asset.js';
import type { ComponentAssetSource } from './manifest.js';
import { registerComponentAssetGraph } from './registration.js';
import { validateComponentAssetGraph } from './validate-graph.js';

const assetModulePromises = new Map<string, Promise<unknown>>();

/** Import, validate, and atomically register one component asset graph. */
export function loadComponentAsset(
  tag: string,
  source: ComponentAssetSource,
): Promise<void> {
  if (typeof source === 'function') {
    return Promise.resolve()
      .then(source)
      .then(imported => registerRootAsset(
        tag,
        `component asset loader for <${tag}>`,
        imported,
      ));
  }
  const assetUrl = new URL(source, document.baseURI);
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
  validateAsset(asset);
  if (asset.root !== expectedRoot) {
    throw new Error(
      `[WebUI] Component asset manifest expected <${expectedRoot}> but ${href} exports <${String(asset.root)}>.`,
    );
  }
  await registerComponentAssetGraph(
    asset,
    prepareAssetComponentStyles,
    validateComponentAssetGraph,
  );
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

/** Validate and atomically register an already imported component asset graph. */
export function registerComponentAsset(asset: ComponentAsset): Promise<void> {
  return registerRootAsset(
    typeof asset.root === 'string' ? asset.root : '',
    'generated component asset',
    { default: asset },
  );
}
