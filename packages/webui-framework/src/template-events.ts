// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Template registration bridge shared by framework and router.
 *
 * `@microsoft/webui-router` stays framework-independent and dispatches this DOM
 * event after registering compiled template data. The framework listens for the
 * event and defines compiler-owned dormant hosts.
 */

import type { TemplateMeta } from './template.js';

/** DOM event emitted when WebUI template data becomes available at runtime. */
export const TEMPLATES_REGISTERED_EVENT = 'webui:templates-registered';

/**
 * Notify optional runtimes that templates have been registered.
 *
 * Styles are deliberately not announced here. This dispatcher runs only after
 * the framework has already accepted them, so repeating them would ask every
 * listener to register the same catalog twice. The router has its own
 * dispatcher and *does* attach styles, but only when the framework bridge was
 * absent — which is why {@link templateRegistrationDetail} still reads them.
 */
export function dispatchTemplatesRegistered(
  templates: Record<string, TemplateMeta>,
): void {
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
export function templateRegistrationDetail(event: Event): {
  templates?: Record<string, TemplateMeta>;
  componentStyles?: unknown;
} | undefined {
  const detail = (event as CustomEvent<{
    templates?: unknown;
    componentStyles?: unknown;
  }>).detail;
  if (!detail || typeof detail !== 'object') return undefined;
  const templates = detail.templates;
  const payload = {
    templates: typeof templates === 'object' && templates !== null
      ? templates as Record<string, TemplateMeta>
      : undefined,
    componentStyles: detail.componentStyles,
  };
  return payload.templates || payload.componentStyles ? payload : undefined;
}
