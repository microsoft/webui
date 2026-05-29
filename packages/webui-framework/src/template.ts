// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Template registry — stores compiled metadata objects from the Rust compiler.
 *
 * Each entry is a metadata object with:
 * - `h`  — static HTML for the component template
 * - `tx` — text runs `[slot, parts]` for text binding positions
 * - `a`  — attribute binding metadata
 * - `ag` — attribute target groups `[path, startIndex, count]`
 * - `c`  — conditional blocks `[conditionAst, blockIndex]`
 * - `cl` — conditional anchor slots
 * - `r`  — repeat/for blocks `[collection, itemVar, blockIndex]`
 * - `rl` — repeat anchor slots
 * - `e`  — element events `[eventName, handlerName, argSpecs, targetPath]`
 * - `el` — event target element paths
 * - `b`  — nested compiled block metadata
 * - `sa` — adopted stylesheet specifier for CSS module strategy
 * - `sd` — shadow DOM flag for client-created components
 * - `re` — root events on the host element
 */

export type {
  CompiledAttrGroupMeta,
  CompiledAttrMeta,
  CompiledAttrPart,
  CompiledCondition,
  CompiledConditionalMeta,
  CompiledEventArg,
  CompiledEventArgs,
  CompiledTextRunMeta,
  TemplateBlockMeta,
  TemplateMeta,
  TemplateNodePath,
  TemplateSlotPath,
} from './template-types.js';

import type { TemplateMeta } from './template-types.js';

declare global {
  interface Window {
    /** Consolidated SSR bootstrap object — single script block. */
    __webui?: {
      state?: Record<string, unknown>;
      templates?: Record<string, TemplateMeta>;
      [key: string]: unknown;
    };
  }
}

export function getTemplate(name: string): TemplateMeta | undefined {
  return window.__webui?.templates?.[name];
}
