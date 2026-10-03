// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { ComponentAsset } from './component-asset/asset.js';
import { registerComponentAssetGraph } from './component-asset/registration.js';
import { takeGeneratedComponentAssetStyles } from './component-asset/generated-manifest.js';
import { preloadComponentAssetStyles as preloadComponentAssetStylesInternal } from './element/link-styles.js';

/** Runtime facade for an imported compiler-generated component asset root. */
export interface GeneratedComponentAsset {
  /** Start graph registration and stylesheet readiness work. */
  preload(): Promise<void>;
  /** Create the root element after its compiled graph and styles are ready. */
  create(): Promise<HTMLElement>;
}

const facades = new WeakMap<ComponentAsset, GeneratedComponentAsset>();

/** Start compiler-known Link stylesheet preloads before importing a generated root. */
export function preloadComponentAssetStyles(root: string): void {
  const styles = takeGeneratedComponentAssetStyles(root);
  if (styles) preloadComponentAssetStylesInternal(styles);
}

/** Return one shared runtime facade for an imported generated root payload. */
export function defineComponentAsset(asset: ComponentAsset): GeneratedComponentAsset {
  if (asset.type !== 'webui-component-asset' || asset.version !== 4) {
    throw new Error('[WebUI] Expected a version 4 component asset root. Rebuild generated inputs with a compatible compiler.');
  }
  const existing = facades.get(asset);
  if (existing) return existing;
  const root = asset.root;
  let pending: Promise<void> | undefined;

  const preload = (): Promise<void> => {
    if (pending) return pending;
    preloadComponentAssetStyles(root);
    const next = registerComponentAssetGraph(asset).catch((error: unknown) => {
      if (pending === next) pending = undefined;
      throw error;
    });
    pending = next;
    return next;
  };

  const create = async (): Promise<HTMLElement> => {
    await preload();
    return document.createElement(root);
  };

  const facade = { preload, create };
  facades.set(asset, facade);
  return facade;
}
