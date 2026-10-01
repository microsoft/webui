// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { ComponentAsset } from './component-asset/asset.js';
import { registerComponentAsset } from './component-asset/loader.js';
import { takeGeneratedComponentAssetStyles } from './component-asset/generated-manifest.js';
import { preloadComponentAssetStyles } from './element/link-styles.js';

/** Runtime API exported by a compiler-generated component asset root. */
export interface GeneratedComponentAsset {
  /** Start graph registration and stylesheet readiness work. */
  preload(): Promise<void>;
  /** Create the root element after its compiled graph and styles are ready. */
  create(): Promise<HTMLElement>;
}

/** Create the small runtime facade embedded by generated component asset roots. */
export function defineComponentAsset(asset: ComponentAsset): GeneratedComponentAsset {
  const root = asset.root ?? '';
  let pending: Promise<void> | undefined;

  const preload = (): Promise<void> => {
    if (pending) return pending;
    const styles = takeGeneratedComponentAssetStyles(root);
    if (styles) preloadComponentAssetStyles(styles);
    const next = registerComponentAsset(asset).catch((error: unknown) => {
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

  return { preload, create };
}
