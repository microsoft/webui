// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  getTemplate,
  prepareAssetTemplateData,
  registerTemplateData,
} from '../template.js';
import {
  hasRegisteredComponentStyleResource,
  registerPreparedComponentStyles,
  validateComponentStylesRegistration,
  type ComponentStyles,
} from '../element/styles.js';
import {
  prepareComponentStyleLinks,
  prepareRegisteredLinkStyles,
} from '../element/link-styles.js';
import type { ComponentAsset } from './asset.js';

/** One asset of a graph, with its style catalog and stylesheet readiness. */
export interface PreparedComponentAsset {
  asset: ComponentAsset;
  componentStyles: ComponentStyles;
  linkStyles?: Promise<void>;
}

type PrepareStyles = (value: unknown) => ComponentStyles;

/** Cross-asset consistency checks applied before any registry mutation. */
export type ValidateComponentAssetGraph = (
  graph: readonly PreparedComponentAsset[],
) => void;

/**
 * Atomically register an already validated or compiler-generated asset graph.
 *
 * Checks that depend only on the graph's own contents are already guaranteed
 * by the compiler that emitted it, so callers that trust their source omit
 * `validateGraph`. Omitting it is what keeps `validate-graph.ts` off the
 * generated path's module graph, so bundlers drop those checks from the
 * startup bundle without relying on a build-time flag. Callers that load an
 * asset from an untrusted source pass it explicitly.
 *
 * Checks that depend on what the page has already loaded cannot be proven at
 * build time and always run - see {@link validateAgainstLoadedPage}.
 */
export async function registerComponentAssetGraph(
  asset: ComponentAsset,
  prepareStyles: PrepareStyles = trustedComponentStyles,
  validateGraph?: ValidateComponentAssetGraph,
): Promise<void> {
  const graph: PreparedComponentAsset[] = [];
  for (let i = 0; i < asset.imports.length; i++) {
    const imported = asset.imports[i];
    if (!componentsAlreadyRegistered(imported.components)) {
      graph.push(prepare(imported, prepareStyles));
    }
  }
  graph.push(prepare(asset, prepareStyles));

  validateGraph?.(graph);
  validateAgainstLoadedPage(graph);

  for (let i = 0; i < graph.length; i++) {
    registerPreparedComponentStyles(graph[i].componentStyles);
  }
  const pendingStyles: Promise<void>[] = [];
  for (let i = 0; i < graph.length; i++) {
    const ready = graph[i].linkStyles;
    if (ready) pendingStyles.push(ready);
  }
  if (pendingStyles.length > 0) await Promise.all(pendingStyles);
  for (let i = 0; i < graph.length; i++) {
    const current = graph[i].asset;
    if (!componentsAlreadyRegistered(current.components)) {
      registerTemplateData(current.templates, current.templateFunctions);
    }
  }
}

function prepare(
  asset: ComponentAsset,
  prepareStyles: PrepareStyles,
): PreparedComponentAsset {
  const componentStyles = prepareStyles(asset.componentStyles);
  prepareAssetTemplateData(asset.templates, asset.templateFunctions);
  const templateLinks = prepareRegisteredLinkStyles(asset.templates);
  const componentLinks = prepareComponentStyleLinks(componentStyles);
  const linkStyles =
    templateLinks && componentLinks
      ? Promise.all([templateLinks, componentLinks]).then(() => {})
      : (templateLinks ?? componentLinks);
  return { asset, componentStyles, linkStyles };
}

/**
 * Validate the graph against state the running page already established.
 *
 * A stale deferred asset can disagree with the entry bundle it is loaded into,
 * and no build-time analysis can rule that out, so these checks ship in
 * production. They run before any registry mutation to keep registration
 * atomic.
 */
function validateAgainstLoadedPage(
  graph: readonly PreparedComponentAsset[],
): void {
  const provided = new Set<string>();
  const resources = new Set<string>();
  for (let i = 0; i < graph.length; i++) {
    const prepared = graph[i];
    validateComponentStylesRegistration(prepared.componentStyles);
    const ids = Object.keys(prepared.componentStyles.resources);
    for (let j = 0; j < ids.length; j++) resources.add(ids[j]);
    const components = prepared.asset.components;
    for (let j = 0; j < components.length; j++) provided.add(components[j]);
  }

  for (let i = 0; i < graph.length; i++) {
    const closures = graph[i].componentStyles.closures;
    const roots = Object.keys(closures);
    for (let j = 0; j < roots.length; j++) {
      const root = roots[j];
      const closure = closures[root];
      for (let k = 0; k < closure.length; k++) {
        const id = closure[k];
        if (!resources.has(id) && !hasRegisteredComponentStyleResource(id)) {
          throw new Error(
            `[WebUI] Component style closure "${root}" references missing resource "${id}".`,
          );
        }
      }
    }
  }

  const missing: string[] = [];
  for (let i = 0; i < graph.length; i++) {
    const required = graph[i].asset.requiredComponents;
    for (let j = 0; j < required.length; j++) {
      const component = required[j];
      if (!provided.has(component) && !getTemplate(component) && missing.indexOf(component) < 0) {
        missing.push(component);
      }
    }
  }
  if (missing.length > 0) {
    throw new Error(
      `[WebUI] Component asset is missing required templ${missing.length === 1 ? 'ate' : 'ates'} ${missing.map(tag => `<${tag}>`).join(', ')}. Load the application entry bundle and protocol before deferred component assets.`,
    );
  }
}

function componentsAlreadyRegistered(components: readonly string[]): boolean {
  if (components.length === 0) return false;
  for (let i = 0; i < components.length; i++) {
    if (!getTemplate(components[i])) return false;
  }
  return true;
}

function trustedComponentStyles(value: unknown): ComponentStyles {
  return value as ComponentStyles;
}
