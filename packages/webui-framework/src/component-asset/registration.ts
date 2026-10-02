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
  sameComponentStyleClosure,
  sameComponentStyleResource,
  validateComponentStylesRegistration,
  type ComponentStyleResource,
  type ComponentStyles,
} from '../element/styles.js';
import {
  prepareComponentStyleLinks,
  prepareRegisteredLinkStyles,
} from '../element/link-styles.js';
import type { ComponentAsset } from './asset.js';

interface PreparedComponentAsset {
  asset: ComponentAsset;
  componentStyles: ComponentStyles;
  linkStyles?: Promise<void>;
}

type PrepareStyles = (value: unknown) => ComponentStyles;

/** Atomically register an already validated or compiler-generated asset graph. */
export async function registerComponentAssetGraph(
  asset: ComponentAsset,
  prepareStyles: PrepareStyles = trustedComponentStyles,
): Promise<void> {
  const graph: PreparedComponentAsset[] = [];
  for (let i = 0; i < asset.imports.length; i++) {
    const imported = asset.imports[i];
    if (!componentsAlreadyRegistered(imported.components)) {
      graph.push(prepare(imported, prepareStyles));
    }
  }
  graph.push(prepare(asset, prepareStyles));
  validateGraph(graph);

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

function validateGraph(graph: readonly PreparedComponentAsset[]): void {
  const resources = new Map<string, ComponentStyleResource>();
  const closures = new Map<string, readonly string[]>();
  const provided = new Set<string>();
  for (let i = 0; i < graph.length; i++) {
    const prepared = graph[i];
    validateComponentStylesRegistration(prepared.componentStyles);
    for (let j = 0; j < prepared.asset.components.length; j++) {
      provided.add(prepared.asset.components[j]);
    }
    collectStyles(prepared.componentStyles, resources, closures);
  }
  validateClosures(graph, resources);
  validateRequiredComponents(graph, provided);
}

function collectStyles(
  styles: ComponentStyles,
  resources: Map<string, ComponentStyleResource>,
  closures: Map<string, readonly string[]>,
): void {
  const resourceIds = Object.keys(styles.resources);
  for (let i = 0; i < resourceIds.length; i++) {
    const id = resourceIds[i];
    const resource = styles.resources[id];
    const current = resources.get(id);
    if (current && !sameComponentStyleResource(current, resource)) {
      throw new Error(`[WebUI] Conflicting component style resource "${id}".`);
    }
    resources.set(id, resource);
  }
  const roots = Object.keys(styles.closures);
  for (let i = 0; i < roots.length; i++) {
    const root = roots[i];
    const closure = styles.closures[root];
    const current = closures.get(root);
    if (current && !sameComponentStyleClosure(current, closure)) {
      throw new Error(`[WebUI] Conflicting component style closure "${root}".`);
    }
    closures.set(root, closure);
  }
}

function validateClosures(
  graph: readonly PreparedComponentAsset[],
  resources: ReadonlyMap<string, ComponentStyleResource>,
): void {
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
}

function validateRequiredComponents(
  graph: readonly PreparedComponentAsset[],
  provided: ReadonlySet<string>,
): void {
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
