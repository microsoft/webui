// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Static component asset loader for the WebUI Framework plugin.
 *
 * Kept outside the framework root entrypoint so apps that only hydrate normal
 * WebUI components do not load this optional CDN/static-asset helper.
 */

import { loadComponentAsset } from './component-asset/loader.js';
import { takeGeneratedComponentAssetStyles } from './component-asset/generated-manifest.js';
import { preloadComponentAssetStyles } from './element/link-styles.js';
import type {
  ComponentAssetCreateOptions,
  ComponentAssetManifest,
  ComponentAssetPreload,
  ComponentAssetRegistry,
  ComponentAssetSource,
  ComponentAssetState,
} from './component-asset/manifest.js';

export type {
  ComponentAsset,
  ComponentAssetImport,
} from './component-asset/asset.js';
export type {
  ComponentAssetCreateOptions,
  ComponentAssetManifest,
  ComponentAssetManifestEntry,
  ComponentAssetPreload,
  ComponentAssetRegistry,
  ComponentAssetSource,
  ComponentAssetState,
} from './component-asset/manifest.js';

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
    const styles = takeGeneratedComponentAssetStyles(tag);
    if (styles) {
      preloadComponentAssetStyles(styles);
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

  async function create(
    tag: string,
    options: ComponentAssetCreateOptions = {},
  ): Promise<HTMLElement> {
    const pending = preload(tag);
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
