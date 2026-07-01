// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Template registration bridge shared by framework and router.
 *
 * `@microsoft/webui-router` must stay platform independent and cannot import the
 * framework. It dispatches this DOM event after registering WebUI template data;
 * the framework listens for the event and can claim HTML-only auto-elements.
 */

import type { TemplateMeta } from './template.js';

/** DOM event emitted when WebUI template data becomes available at runtime. */
export const TEMPLATES_REGISTERED_EVENT = 'webui:templates-registered';

/**
 * Notify optional runtimes that templates have been registered.
 *
 * The payload is intentionally generic so consumers can decide what to do
 * without creating package dependencies between router and framework. Routers
 * may include `blockedTags` for tags owned by lazy component loaders; automatic
 * template runtimes must not claim those tags before the loader module defines
 * the authored element.
 */
export function dispatchTemplatesRegistered(
  templates: Record<string, TemplateMeta>,
  blockedTags?: readonly string[],
): void {
  if (
    typeof window === 'undefined' ||
    typeof CustomEvent !== 'function' ||
    typeof window.dispatchEvent !== 'function'
  ) {
    return;
  }

  window.dispatchEvent(new CustomEvent(TEMPLATES_REGISTERED_EVENT, {
    detail: { templates, blockedTags },
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

/** Read the optional loader-owned tag list from a template registration event. */
export function templateRegistrationBlockedTags(event: Event): readonly string[] | undefined {
  const detail = (event as CustomEvent<{ blockedTags?: unknown }>).detail;
  const blockedTags = detail?.blockedTags;
  if (!Array.isArray(blockedTags)) return undefined;
  for (let i = 0; i < blockedTags.length; i++) {
    if (typeof blockedTags[i] !== 'string') return undefined;
  }
  return blockedTags as readonly string[];
}
