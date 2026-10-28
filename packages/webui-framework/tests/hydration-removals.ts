// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { Page } from '@playwright/test';

/**
 * Record nodes removed from a component's SSR render roots while its
 * definition upgrades and hydrates them.
 *
 * WebUI hydrates synchronously inside `customElements.define()`, so observing
 * only that call isolates hydration from later updates. Hydration may remove
 * its comment markers; removing any other node detaches adopted SSR content.
 */
export async function recordHydrationRemovals(page: Page, tagName: string): Promise<void> {
  await page.addInitScript((tag) => {
    const removed: string[] = [];
    Object.defineProperty(window, '__hydrationRemovals', { value: removed });
    const registry = CustomElementRegistry.prototype;
    const define = registry.define;
    registry.define = function (name, constructor, options) {
      if (name !== tag) {
        define.call(this, name, constructor, options);
        return;
      }
      const observer = new MutationObserver(() => {});
      for (const host of Array.from(document.querySelectorAll(tag))) {
        observer.observe(host.shadowRoot ?? host, { childList: true, subtree: true });
      }
      try {
        define.call(this, name, constructor, options);
      } finally {
        for (const record of observer.takeRecords()) {
          for (const node of Array.from(record.removedNodes)) {
            if (node.nodeType !== Node.COMMENT_NODE) removed.push(node.nodeName);
          }
        }
        observer.disconnect();
      }
    };
  }, tagName);
}

/** Wait for `tagName` to hydrate, then read the removals recorded for it. */
export async function hydrationRemovals(page: Page, tagName: string): Promise<string[]> {
  await page.waitForFunction((tag) => {
    const host = document.querySelector(tag) as unknown as { $ready?: boolean } | null;
    return host?.$ready === true;
  }, tagName);
  return page.evaluate(() =>
    (window as unknown as { __hydrationRemovals: string[] }).__hydrationRemovals);
}
