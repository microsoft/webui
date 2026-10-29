// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Compiler-emitted asset references (component stylesheets, the bootstrap
 * `css` list, and lazy-region packs) are relative to the asset base. A server
 * whose assets live on a host known only at runtime publishes that base as
 * `window.__webui.assetBase`, and in the document head as
 * `<meta name="webui-asset-base">` so it applies before the runtime reads its
 * bootstrap data; otherwise the document base applies. URLs that authors write
 * in templates never pass through here.
 */

let configured: string | undefined;
let documentBase: string | undefined;
let base: string | undefined;
/** Each document's asset base meta, read once. */
const published = new WeakMap<Document, string | undefined>();

/**
 * Resolve a compiler-emitted, base-relative asset path to an absolute URL.
 *
 * Returns `path` unchanged when there is no document or it does not resolve.
 */
export function resolveAsset(path: string, owner?: Document): string {
  try {
    owner ??= document;
    const value = (owner.defaultView ?? globalThis.window)?.__webui?.assetBase ?? publishedBase(owner);
    const baseURI = owner.baseURI;
    if (value !== configured || baseURI !== documentBase) {
      // The configured base may itself be relative to the document.
      base = value ? new URL(value, baseURI).href : baseURI;
      configured = value;
      documentBase = baseURI;
    }
    return new URL(path, base).href;
  } catch {
    return path;
  }
}

function publishedBase(owner: Document): string | undefined {
  if (!published.has(owner)) {
    published.set(owner, owner.querySelector<HTMLMetaElement>('meta[name="webui-asset-base"]')?.content);
  }
  return published.get(owner);
}
