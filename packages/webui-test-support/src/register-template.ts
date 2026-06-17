// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Minimal template registration utilities for E2E test fixtures.
 *
 * Most fixtures now use real WebUI HTML templates compiled by the pipeline
 * (see fixture-render.ts). These helpers remain for edge cases that need
 * programmatic template registration (e.g. light-DOM hydration tests,
 * client-created component tests).
 */

import type {
  CompiledConditionFn,
  TemplateMeta,
} from '../../webui-framework/src/template-types.js';

export type { CompiledConditionFn, TemplateMeta };

/**
 * Register a compiled template so the framework can hydrate or mount
 * a custom element with the given tag name.
 */
export function registerCompiledTemplate(
  name: string,
  meta: TemplateMeta,
  fns?: CompiledConditionFn[],
): void {
  const w = window as unknown as {
    __webui?: {
      templates?: Record<string, TemplateMeta>;
      templateFns?: Record<string, CompiledConditionFn[]>;
      [k: string]: unknown;
    };
  };
  if (!w.__webui) w.__webui = {};
  if (!w.__webui.templates) w.__webui.templates = {};
  w.__webui.templates[name] = meta;
  if (fns) {
    if (!w.__webui.templateFns) w.__webui.templateFns = {};
    w.__webui.templateFns[name] = fns;
  }
}

/** Render a static template registration as an inline `<script>` tag. */
export function renderTemplateScript(name: string, meta: TemplateMeta): string {
  return `<script>window.__webui=window.__webui||{};window.__webui.templates=window.__webui.templates||{};window.__webui.templates[${JSON.stringify(name)}]=${JSON.stringify(meta)};</script>`;
}
