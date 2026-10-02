// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Within-graph consistency checks for a prepared component asset graph.
 *
 * Two assets of one graph are always emitted by one compiler run, so the
 * compiler already guarantees they agree. This module is therefore kept off
 * the generated path's static import graph and exists for callers that load
 * manifest-driven assets from an untrusted source. Checks that depend on the
 * running page instead live in `registration.ts` and always ship.
 */

import {
  sameComponentStyleClosure,
  sameComponentStyleResource,
  type ComponentStyleResource,
} from '../element/styles.js';
import type { PreparedComponentAsset } from './registration.js';

/** Validate that assets of one graph declare identical shared styles. */
export function validateComponentAssetGraph(
  graph: readonly PreparedComponentAsset[],
): void {
  const resources = new Map<string, ComponentStyleResource>();
  const closures = new Map<string, readonly string[]>();
  for (let i = 0; i < graph.length; i++) {
    const styles = graph[i].componentStyles;
    const resourceIds = Object.keys(styles.resources);
    for (let j = 0; j < resourceIds.length; j++) {
      const id = resourceIds[j];
      const resource = styles.resources[id];
      const current = resources.get(id);
      if (current && !sameComponentStyleResource(current, resource)) {
        throw new Error(`[WebUI] Conflicting component style resource "${id}".`);
      }
      resources.set(id, resource);
    }
    const roots = Object.keys(styles.closures);
    for (let j = 0; j < roots.length; j++) {
      const root = roots[j];
      const closure = styles.closures[root];
      const current = closures.get(root);
      if (current && !sameComponentStyleClosure(current, closure)) {
        throw new Error(`[WebUI] Conflicting component style closure "${root}".`);
      }
      closures.set(root, closure);
    }
  }
}
