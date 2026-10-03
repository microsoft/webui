// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  getTemplate,
  prepareAssetTemplateData,
  registerTemplateData,
} from '../template.js';
import {
  registerPreparedComponentStyles,
  validateComponentStylesRegistration,
  type ComponentStyles,
} from '../element/styles.js';
import {
  prepareComponentStyleLinks,
  prepareRegisteredLinkStyles,
} from '../element/link-styles.js';
import type { ComponentAsset, ComponentAssetPayload } from './asset.js';

/** A prepared graph member, with optional template payload and CSS readiness. */
export interface PreparedComponentAsset {
  tag: string;
  payload?: ComponentAssetPayload;
  componentStyles: ComponentStyles;
}

type PrepareStyles = (value: unknown) => ComponentStyles;

/** Additional integrity checks for manifest-driven, untrusted input. */
export type ValidateComponentAssetGraph = (
  graph: readonly PreparedComponentAsset[],
) => void;

/** Register compiler-proven coverage, checking live page conflicts atomically. */
export async function registerComponentAssetGraph(
  asset: ComponentAsset,
  prepareStyles: PrepareStyles = trustedComponentStyles,
  validateGraph?: ValidateComponentAssetGraph,
): Promise<void> {
  const graph: PreparedComponentAsset[] = [];
  for (let i = 0; i < asset.imports.length; i++) {
    const payload = asset.imports[i];
    const styles = prepareStyles(payload.componentStyles);
    validateComponentStylesRegistration(styles);
    graph.push({
      tag: Object.keys(payload.templates)[0],
      payload,
      componentStyles: styles,
    });
  }
  const rootStyles = prepareStyles(asset.componentStyles);
  validateComponentStylesRegistration(rootStyles);
  graph.push({ tag: asset.root, componentStyles: rootStyles });
  validateGraph?.(graph);
  validateExternalTemplates(asset.externalComponents);

  for (let i = 0; i < graph.length; i++) {
    const prepared = graph[i];
    const payload = prepared.payload;
    if (payload && !getTemplate(prepared.tag)) {
      prepareAssetTemplateData(payload.templates, payload.templateFunctions);
    }
  }
  const pendingStyles: Promise<void>[] = [];
  for (let i = 0; i < graph.length; i++) {
    const prepared = graph[i];
    const payload = prepared.payload;
    const templateLinks = payload && prepareRegisteredLinkStyles(payload.templates);
    const componentLinks = prepareComponentStyleLinks(prepared.componentStyles);
    if (templateLinks) pendingStyles.push(templateLinks);
    if (componentLinks) pendingStyles.push(componentLinks);
  }
  for (let i = 0; i < graph.length; i++) {
    registerPreparedComponentStyles(graph[i].componentStyles);
  }
  if (pendingStyles.length > 0) await Promise.all(pendingStyles);
  for (let i = 0; i < graph.length; i++) {
    const payload = graph[i].payload;
    if (payload && !getTemplate(graph[i].tag)) {
      registerTemplateData(payload.templates, payload.templateFunctions);
    }
  }
}

function validateExternalTemplates(external: readonly string[]): void {
  const missing: string[] = [];
  for (let i = 0; i < external.length; i++) {
    if (!getTemplate(external[i])) missing.push(external[i]);
  }
  if (missing.length > 0) {
    throw new Error(
      `[WebUI] Component asset requires entr${missing.length === 1 ? 'y template' : 'y templates'} ${missing.map(tag => `<${tag}>`).join(', ')}. Load the application entry bundle and protocol before deferred component assets.`,
    );
  }
}

function trustedComponentStyles(value: unknown): ComponentStyles {
  return value as ComponentStyles;
}
