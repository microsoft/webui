// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

type GeneratedComponentAssetStyleManifest = Record<string, readonly string[]>;

const COMPONENT_ASSET_MANIFEST_ID = 'webui-component-assets';
let manifestLoaded = false;

/** Consume compiler-owned component asset styles emitted in the document head. */
export function takeGeneratedComponentAssetStyles(
  tag: string,
): readonly string[] | undefined {
  if (typeof window !== 'object' || typeof document !== 'object') return undefined;
  const existing = window.__webui?.componentAssetStyles;
  if (existing) {
    const styles = existing[tag];
    if (styles) delete existing[tag];
    return styles;
  }
  if (manifestLoaded) return undefined;

  const element = document.getElementById(COMPONENT_ASSET_MANIFEST_ID);
  if (!element) {
    manifestLoaded = true;
    return undefined;
  }
  const text = element.textContent;
  element.remove();
  const manifest = text
    ? JSON.parse(text) as GeneratedComponentAssetStyleManifest
    : {};
  const runtime = window.__webui ?? (window.__webui = {});
  runtime.componentAssetStyles = manifest;
  manifestLoaded = true;
  const styles = manifest[tag];
  if (styles) delete manifest[tag];
  return styles;
}
