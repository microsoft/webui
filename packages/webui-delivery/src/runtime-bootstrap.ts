// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

export type RuntimeBootstrap = WebUIRuntimeGlobal & {
  templates?: Record<string, unknown>;
  templateFns?: Record<string, unknown>;
  componentAssetStyles?: Record<string, readonly string[]>;
};

let loaded = false;

export function loadRuntimeBootstrap(): void {
  const runtime = window.__webui as RuntimeBootstrap | undefined;
  if (loaded || runtime?.state !== undefined) return;
  const element = document.getElementById('webui-data');
  if (!element) { loaded = true; return; }
  const text = element.textContent;
  if (text) {
    const parsed = JSON.parse(text) as RuntimeBootstrap;
    if (runtime?.buildId && parsed.buildId && runtime.buildId !== parsed.buildId) {
      throw new Error('[WebUI] Resources use different builds. Reload the document.');
    }
    if (runtime?.templateFns) parsed.templateFns = runtime.templateFns;
    if (runtime?.componentAssetStyles) parsed.componentAssetStyles = runtime.componentAssetStyles;
    window.__webui = parsed;
  }
  element.remove();
  loaded = true;
}
