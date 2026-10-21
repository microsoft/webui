// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { TemplateMeta } from './template-types.js';

export interface TemplateCacheHooks {
  accept(templates: Record<string, TemplateMeta>): void;
  retain(tag: string, meta: TemplateMeta): TemplateMeta;
  lookup(tag: string): TemplateMeta | undefined;
  hydrated(element: Element): void;
  visitRetained(
    cached: Record<string, TemplateMeta> | undefined,
    visit: (tag: string, meta: TemplateMeta) => void,
  ): void;
}

export let templateCacheHooks: TemplateCacheHooks | undefined;
let consumedWithoutCache = false;

export function noteTemplateRead(): void {
  if (!templateCacheHooks) consumedWithoutCache = true;
}

export function installTemplateCacheHooks(hooks: TemplateCacheHooks): void {
  if (templateCacheHooks === hooks) return;
  if (templateCacheHooks || consumedWithoutCache) {
    throw new Error(
      '[WebUI] Import @microsoft/webui-framework/template-cache.js before component definitions ' +
      'and share one framework instance. Template ownership cannot be reconstructed after use.',
    );
  }
  templateCacheHooks = hooks;
}
