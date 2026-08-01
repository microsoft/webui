// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type {
  CompiledConditionFn,
  TemplateMeta,
} from '../template.js';

const ASSET_TYPE = 'webui-component-asset';
const ASSET_VERSION = 2;

/** Dynamic import edge from a component asset root to a shared chunk. */
export interface ComponentAssetImport {
  /** Component templates provided by the imported chunk. */
  components: string[];
  /** Fully resolved chunk URL used for concurrent-load deduplication. */
  href: string;
  /** Browser-native dynamic import emitted into the root asset module. */
  load: () => Promise<unknown>;
}

/** WebUI Framework component asset emitted by `webui build --emit-component-assets`. */
export interface ComponentAsset {
  type: 'webui-component-asset';
  version: 2;
  kind: 'root' | 'chunk';
  root?: string;
  components: string[];
  requiredComponents: string[];
  externalComponents: string[];
  imports: ComponentAssetImport[];
  templateStyles: string[];
  templates: Record<string, TemplateMeta>;
  templateFunctions?: Record<string, CompiledConditionFn[]>;
}

/** Read the default payload exported by an imported component asset module. */
export function readComponentAssetModule(module: unknown): unknown {
  if (!isObject(module) || !isObject(module.default)) {
    throw new Error('[WebUI] Component asset module must default-export an asset object.');
  }
  return module.default;
}

/** Validate the complete static contract for one emitted component asset. */
export function validateAsset(
  value: unknown,
  expectedKind: ComponentAsset['kind'],
): asserts value is ComponentAsset {
  if (!isObject(value)) {
    throw new Error('[WebUI] Component asset default export must be an object.');
  }
  const asset = value;
  if (asset.type !== ASSET_TYPE) {
    throw new Error(`[WebUI] Invalid component asset type: ${String(asset.type)}`);
  }
  if (asset.version !== ASSET_VERSION) {
    throw new Error(`[WebUI] Unsupported component asset version: ${String(asset.version)}`);
  }
  if (asset.kind !== expectedKind) {
    throw new Error(
      `[WebUI] Expected component asset kind "${expectedKind}", received "${String(asset.kind)}".`,
    );
  }
  validateUniqueStringArray(asset.components, 'components');
  validateUniqueStringArray(asset.requiredComponents, 'requiredComponents');
  validateUniqueStringArray(asset.externalComponents, 'externalComponents');
  if (!Array.isArray(asset.imports)) {
    throw new Error('[WebUI] Component asset imports must be an array.');
  }
  validateStringArray(asset.templateStyles, 'templateStyles');
  if (!isObject(asset.templates)) {
    throw new Error('[WebUI] Component asset templates must be an object.');
  }

  let root: string | undefined;
  if (expectedKind === 'root') {
    if (typeof asset.root !== 'string' || asset.root.length === 0) {
      throw new Error('[WebUI] Component asset root must name its root component.');
    }
    root = asset.root;
  } else {
    validateChunkShape(asset.root, asset.imports, asset.externalComponents);
  }

  for (let i = 0; i < asset.imports.length; i++) {
    validateAssetImport(asset.imports[i]);
  }
  validateAssetPayload(
    asset.components,
    asset.templates,
    asset.templateFunctions,
  );
  validateAssetCoverage(
    root,
    asset.components,
    asset.requiredComponents,
    asset.externalComponents,
    asset.imports,
  );
}

/** Return whether two ordered component declarations are identical. */
export function sameComponents(left: readonly string[], right: readonly string[]): boolean {
  if (left.length !== right.length) return false;
  for (let i = 0; i < left.length; i++) {
    if (left[i] !== right[i]) return false;
  }
  return true;
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function validateChunkShape(
  root: unknown,
  imports: readonly unknown[],
  externalComponents: readonly string[],
): void {
  if (root !== undefined) {
    throw new Error('[WebUI] Shared component asset chunks cannot declare a root.');
  }
  if (imports.length !== 0) {
    throw new Error('[WebUI] Shared component asset chunks cannot import other chunks.');
  }
  if (externalComponents.length !== 0) {
    throw new Error('[WebUI] Shared component asset chunks cannot declare external components.');
  }
}

function validateAssetImport(value: unknown): asserts value is ComponentAssetImport {
  if (!isObject(value)) {
    throw new Error('[WebUI] Component asset import must be an object.');
  }
  const assetImport = value;
  validateUniqueStringArray(assetImport.components, 'import components');
  if (assetImport.components.length === 0) {
    throw new Error('[WebUI] Component asset imports must provide at least one component.');
  }
  if (typeof assetImport.href !== 'string' || assetImport.href.length === 0) {
    throw new Error('[WebUI] Component asset import href must be a non-empty string.');
  }
  if (typeof assetImport.load !== 'function') {
    throw new Error('[WebUI] Component asset import load must be a function.');
  }
}

function validateUniqueStringArray(value: unknown, field: string): asserts value is string[] {
  validateStringArray(value, field);
  const seen = new Set<string>();
  for (let i = 0; i < value.length; i++) {
    if (seen.has(value[i])) {
      throw new Error(`[WebUI] Component asset ${field} cannot contain duplicate <${value[i]}> entries.`);
    }
    seen.add(value[i]);
  }
}

function validateStringArray(value: unknown, field: string): asserts value is string[] {
  if (!Array.isArray(value)) {
    throw new Error(`[WebUI] Component asset ${field} must be an array.`);
  }
  for (let i = 0; i < value.length; i++) {
    if (typeof value[i] !== 'string' || value[i].length === 0) {
      throw new Error(`[WebUI] Component asset ${field} must contain non-empty strings.`);
    }
  }
}

function validateAssetCoverage(
  root: string | undefined,
  components: readonly string[],
  requiredComponents: readonly string[],
  externalComponents: readonly string[],
  imports: readonly ComponentAssetImport[],
): void {
  const required = new Set(requiredComponents);
  if (root && !required.has(root)) {
    throw new Error(
      `[WebUI] Component asset root <${root}> must include itself in requiredComponents.`,
    );
  }

  const providers = new Set<string>();
  const addProvider = (component: string): void => {
    if (!required.has(component)) {
      throw new Error(
        `[WebUI] Component asset provides undeclared template <${component}>. Add it to requiredComponents.`,
      );
    }
    if (providers.has(component)) {
      throw new Error(
        `[WebUI] Component asset assigns required template <${component}> to more than one payload, import, or external prerequisite.`,
      );
    }
    providers.add(component);
  };

  for (let i = 0; i < components.length; i++) addProvider(components[i]);
  for (let i = 0; i < externalComponents.length; i++) addProvider(externalComponents[i]);
  for (let i = 0; i < imports.length; i++) {
    const imported = imports[i].components;
    for (let j = 0; j < imported.length; j++) addProvider(imported[j]);
  }
  for (let i = 0; i < requiredComponents.length; i++) {
    const component = requiredComponents[i];
    if (!providers.has(component)) {
      throw new Error(
        `[WebUI] Component asset required template <${component}> has no payload, import, or external prerequisite.`,
      );
    }
  }
}

function validateAssetPayload(
  components: readonly string[],
  templates: Record<string, unknown>,
  templateFunctions: unknown,
): void {
  const declared = new Set(components);
  const templateNames = Object.keys(templates);
  for (let i = 0; i < templateNames.length; i++) {
    const template = templateNames[i];
    if (!declared.has(template)) {
      throw new Error(
        `[WebUI] Component asset templates contain undeclared payload <${template}>.`,
      );
    }
  }
  for (let i = 0; i < components.length; i++) {
    const component = components[i];
    if (!Object.prototype.hasOwnProperty.call(templates, component)) {
      throw new Error(
        `[WebUI] Component asset payload <${component}> is missing its template metadata.`,
      );
    }
  }

  if (templateFunctions === undefined) return;
  if (!isObject(templateFunctions)) {
    throw new Error('[WebUI] Component asset templateFunctions must be an object.');
  }
  const functionNames = Object.keys(templateFunctions);
  for (let i = 0; i < functionNames.length; i++) {
    const component = functionNames[i];
    if (!declared.has(component)) {
      throw new Error(
        `[WebUI] Component asset templateFunctions contains undeclared payload <${component}>.`,
      );
    }
    const functions = templateFunctions[component];
    if (
      !Array.isArray(functions) ||
      functions.some(candidate => typeof candidate !== 'function')
    ) {
      throw new Error(
        `[WebUI] Component asset templateFunctions for <${component}> must contain only functions.`,
      );
    }
  }
}
