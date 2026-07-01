// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Template root metadata helpers.
 *
 * The compiler emits component-level template roots (`tr`), observed host
 * attributes (`ta`), and feature flags (`tf`) directly into `TemplateMeta`.
 * The browser runtime only normalizes those compact arrays into lookup tables;
 * it no longer scans every binding to rediscover metadata the compiler already
 * knew at build time.
 */

import type { TemplateMeta } from './template.js';

const TEMPLATE_FEATURE_EVENTS = 1;

const EMPTY_ROOTS: readonly string[] = Object.freeze([]);

/** Collect component-level state roots referenced by a template. */
export function collectTemplateRoots(meta: TemplateMeta): readonly string[] {
  return meta.tr ?? EMPTY_ROOTS;
}

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
  const attrs = meta.ta ?? EMPTY_ROOTS;
  for (let i = 0; i + 1 < attrs.length; i += 2) {
    if (attrs[i] === attrName) return attrs[i + 1];
  }
  return undefined;
}

/**
 * Return the observed host attribute for one template root.
 *
 * Auto-elements use this during SSR state seeding to let explicit host
 * attributes win over server state without importing the decorator attr-name
 * conversion code into the HTML-only runtime.
 */
export function templateAttributeForRoot(meta: TemplateMeta, root: string): string | undefined {
  const attrs = meta.ta ?? EMPTY_ROOTS;
  for (let i = 0; i + 1 < attrs.length; i += 2) {
    if (attrs[i + 1] === root) return attrs[i];
  }
  return undefined;
}

/** Return true when the template requires authored JavaScript event handlers. */
export function templateHasEventHandlers(meta: TemplateMeta): boolean {
  return ((meta.tf ?? 0) & TEMPLATE_FEATURE_EVENTS) !== 0;
}

/** Return true when a scriptless template needs a hydrating auto-element. */
export function templateNeedsAutoElement(meta: TemplateMeta): boolean {
  return !!meta.ae && !templateHasEventHandlers(meta) && collectTemplateRoots(meta).length !== 0;
}
