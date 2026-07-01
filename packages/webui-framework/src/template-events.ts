// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { TemplateMeta } from './template.js';

/** Event emitted when compiled template metadata becomes available. */
export const TEMPLATES_REGISTERED_EVENT = 'webui:templates-registered';

/** Notify optional runtimes that compiled templates have been registered. */
export function dispatchTemplatesRegistered(templates: Record<string, TemplateMeta>): void {
  if (
    typeof window === 'undefined' ||
    typeof CustomEvent !== 'function' ||
    typeof window.dispatchEvent !== 'function'
  ) {
    return;
  }

  window.dispatchEvent(new CustomEvent(TEMPLATES_REGISTERED_EVENT, {
    detail: { templates },
  }));
}

/** Read a template registration event payload without trusting arbitrary detail. */
export function templateRegistrationDetail(event: Event): Record<string, TemplateMeta> | undefined {
  const detail = (event as CustomEvent<{ templates?: unknown }>).detail;
  const templates = detail?.templates;
  return typeof templates === 'object' && templates !== null
    ? templates as Record<string, TemplateMeta>
    : undefined;
}
