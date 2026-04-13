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
  TemplateMeta,
} from '../../webui-framework/src/template-types.js';

export type { TemplateMeta };

/**
 * Register a compiled template so the framework can hydrate or mount
 * a custom element with the given tag name.
 */
export function registerCompiledTemplate(
  name: string,
  meta: TemplateMeta,
): void {
  const w = window as unknown as { __webui_templates?: Record<string, TemplateMeta> };
  const templates = w.__webui_templates ?? (w.__webui_templates = {});
  templates[name] = meta;
}

/** Render a template registration as an inline `<script>` tag. */
export function renderTemplateScript(name: string, meta: TemplateMeta): string {
  return `<script>(function(){var w=window.__webui_templates||(window.__webui_templates={});w[${JSON.stringify(name)}]=${JSON.stringify(meta)};})();</script>`;
}

