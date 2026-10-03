// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { CompiledConditionFn, TemplateMeta } from '../template.js';
import {
  requireComponentStyles,
  type ComponentStyles,
} from '../element/styles.js';

const preparedAssetStyles = new WeakMap<object, ComponentStyles>();

/** One statically imported, compiler-owned component payload. */
export interface ComponentAssetPayload {
  componentStyles: ComponentStyles;
  /** Exactly one compiled component template. */
  templates: Record<string, TemplateMeta>;
  templateFunctions?: Record<string, CompiledConditionFn[]>;
}

/** Versioned root emitted by `webui build --emit-component-assets`. */
export interface ComponentAsset {
  type: 'webui-component-asset';
  version: 4;
  root: string;
  externalComponents: string[];
  /** The disjoint, asset-owned portion of the required template closure. */
  imports: ComponentAssetPayload[];
  /** Exact external style resources needed by the imported closures. */
  componentStyles: ComponentStyles;
}

/** Validate and detach a style catalog once for each imported object. */
export function prepareAssetComponentStyles(value: unknown): ComponentStyles {
  if (isObject(value)) {
    const cached = preparedAssetStyles.get(value);
    if (cached) return cached;
    const prepared = requireComponentStyles(value);
    preparedAssetStyles.set(value, prepared);
    return prepared;
  }
  return requireComponentStyles(value);
}

/** Read the default payload exported by an imported root module. */
export function readComponentAssetModule(module: unknown): unknown {
  if (!isObject(module) || !isObject(module.default)) {
    throw new Error('[WebUI] Component asset module must default-export an asset object.');
  }
  return module.default;
}

/** Validate a versioned root and all its compact static payloads. */
export function validateAsset(value: unknown): asserts value is ComponentAsset {
  if (!isObject(value)) {
    throw new Error('[WebUI] Component asset default export must be an object.');
  }
  if (value.type !== 'webui-component-asset') {
    throw new Error(`[WebUI] Invalid component asset type: ${String(value.type)}`);
  }
  if (value.version !== 4) {
    throw new Error(`[WebUI] Unsupported component asset version: ${String(value.version)}`);
  }
  if (typeof value.root !== 'string' || value.root.length === 0) {
    throw new Error('[WebUI] Component asset root must name its root component.');
  }
  if (value.componentStyles === undefined) {
    throw new Error('[WebUI] Version 4 component assets require componentStyles.');
  }
  prepareAssetComponentStyles(value.componentStyles);
  validateExternalComponents(value.externalComponents);
  if (!Array.isArray(value.imports)) {
    throw new Error('[WebUI] Component asset imports must be an array.');
  }
  const providers = new Set(value.externalComponents);
  for (let i = 0; i < value.imports.length; i++) {
    const tag = validatePayload(value.imports[i]);
    if (providers.has(tag)) {
      throw new Error(`[WebUI] Component asset assigns template <${tag}> to more than one import or external prerequisite.`);
    }
    providers.add(tag);
  }
  if (!providers.has(value.root)) {
    throw new Error(`[WebUI] Component asset root <${value.root}> has no imported payload or external prerequisite.`);
  }
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function validateExternalComponents(value: unknown): asserts value is string[] {
  if (!Array.isArray(value)) {
    throw new Error('[WebUI] Component asset externalComponents must be an array.');
  }
  const seen = new Set<string>();
  for (let i = 0; i < value.length; i++) {
    const tag: unknown = value[i];
    if (typeof tag !== 'string' || tag.length === 0 || seen.has(tag)) {
      throw new Error('[WebUI] Component asset externalComponents must contain unique non-empty strings.');
    }
    seen.add(tag);
  }
}

function validatePayload(value: unknown): string {
  if (!isObject(value) || !isObject(value.templates)) {
    throw new Error('[WebUI] Component asset payload must contain its template metadata.');
  }
  const tags = Object.keys(value.templates);
  if (tags.length !== 1 || tags[0].length === 0) {
    throw new Error('[WebUI] Component asset payload must contain exactly one template.');
  }
  const tag = tags[0];
  prepareAssetComponentStyles(value.componentStyles);
  const functions = value.templateFunctions;
  if (functions !== undefined) {
    if (!isObject(functions) || Object.keys(functions).some(name => name !== tag)) {
      throw new Error(`[WebUI] Component asset templateFunctions contains undeclared payload for <${tag}>.`);
    }
    const closures = functions[tag];
    if (!Array.isArray(closures) || closures.some(candidate => typeof candidate !== 'function')) {
      throw new Error(`[WebUI] Component asset templateFunctions for <${tag}> must contain only functions.`);
    }
  }
  return tag;
}
