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
  const roots = meta.tr;
  if (!roots) return false;
  for (let i = 0; i < roots.length; i++) {
    if (roots[i] === root) return true;
  }
  return false;
}

/** Return the template state root observed by one host attribute. */
export function templateRootForAttribute(meta: TemplateMeta, attrName: string): string | undefined {
  const roots = meta.tr;
  const attrs = meta.ta;
  if (!roots || !attrs) return undefined;
  for (let i = 0; i < attrs.length && i < roots.length; i++) {
    if (attrs[i] === attrName) return roots[i];
  }
  return undefined;
}

/** Return true when the compiler owns a dormant TemplateElement host. */
export function templateNeedsStaticHost(meta: TemplateMeta): boolean {
  return !!meta.th;
}
