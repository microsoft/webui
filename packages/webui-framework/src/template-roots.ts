// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Template root metadata helpers.
 *
 * The compiler emits component-level template roots (`tr`), observed host
 * attributes (`ta`, index-aligned with `tr`), and exact dormant-host ownership
 * (`th`) directly into `TemplateMeta`. The browser runtime only normalizes
 * those compact fields instead of rediscovering them from bindings.
 */

import type { TemplateMeta } from './template.js';

/** Return true when a compiler-emitted root list contains `root`. */
export function templateHasRoot(meta: TemplateMeta, root: string): boolean {
  return meta.tr?.includes(root) ?? false;
}

/** Return the template state root observed by one host attribute. */
export function templateRootForAttribute(meta: TemplateMeta, attrName: string): string | undefined {
  const index = meta.ta?.indexOf(attrName) ?? -1;
  return index < 0 ? undefined : meta.tr?.[index];
}

/** Return true when the compiler owns a dormant TemplateElement host. */
export function templateNeedsStaticHost(meta: TemplateMeta): boolean {
  return !meta.f && !!meta.th;
}
