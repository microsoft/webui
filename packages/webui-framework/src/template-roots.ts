// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Template root analysis helpers.
 *
 * The compiled template tells the runtime which state roots are actually read by
 * bindings. `CoreElement` uses that information to keep template-only state
 * hidden when app code omits `@observable` / `@attr`, and auto-elements use it
 * to observe only attributes that can affect DOM output.
 */

import { toKebabCase } from './decorators.js';
import type {
  CompiledAttrPart,
  TemplateBlockMeta,
  TemplateMeta,
} from './template.js';

interface BlockVisit {
  block: TemplateBlockMeta;
  scopes?: ScopeName;
}

interface ScopeName {
  name: string;
  parent?: ScopeName;
}

const templateRootsCache = new WeakMap<TemplateMeta, readonly string[]>();
const templateRootSetCache = new WeakMap<TemplateMeta, ReadonlySet<string>>();
const templateAttributeCache = new WeakMap<TemplateMeta, ReadonlyMap<string, string>>();
const templateEventCache = new WeakMap<TemplateMeta, boolean>();

/** Return the top-level state key for a binding path. */
function pathRoot(path: string): string {
  const dot = path.indexOf('.');
  return dot === -1 ? path : path.slice(0, dot);
}

/** Ignore repeat item aliases so they do not become component state roots. */
function isScopedPath(path: string, scopes?: ScopeName): boolean {
  const root = pathRoot(path);
  let current = scopes;
  while (current) {
    if (current.name === root) return true;
    current = current.parent;
  }
  return false;
}

/** Add a binding path root unless it belongs to the current repeat scope. */
function addRoot(roots: Set<string>, path: string, scopes?: ScopeName): void {
  if (!path || isScopedPath(path, scopes)) return;
  roots.add(pathRoot(path));
}

/** Add all dynamic roots from a mixed text or attribute binding. */
function addPartRoots(roots: Set<string>, parts: CompiledAttrPart[], scopes?: ScopeName): void {
  for (let i = 0; i < parts.length; i++) {
    const part = parts[i];
    if (typeof part !== 'string') addRoot(roots, part[0], scopes);
  }
}

/**
 * Collect component-level state roots referenced by a template.
 *
 * This walks nested condition/repeat block metadata iteratively so deeply nested
 * templates cannot overflow the stack. Repeat item variables are tracked as
 * lexical scopes and excluded from the returned component root set.
 */
export function collectTemplateRoots(meta: TemplateMeta): readonly string[] {
  const cached = templateRootsCache.get(meta);
  if (cached) return cached;

  const roots = new Set<string>();
  const stack: BlockVisit[] = [{ block: meta }];

  while (stack.length > 0) {
    const visit = stack.pop();
    if (!visit) continue;

    const { block, scopes } = visit;
    if (block.tx) {
      for (let i = 0; i < block.tx.length; i++) {
        addPartRoots(roots, block.tx[i][1], scopes);
      }
    }

    if (block.a) {
      for (let i = 0; i < block.a.length; i++) {
        const attr = block.a[i];
        switch (attr[1]) {
          case 0:
          case 1:
            addRoot(roots, attr[2], scopes);
            break;
          case 2:
            for (let j = 0; j < attr[2][1].length; j++) {
              addRoot(roots, attr[2][1][j], scopes);
            }
            break;
          case 3:
            addPartRoots(roots, attr[2], scopes);
            break;
        }
      }
    }

    if (block.c) {
      for (let i = 0; i < block.c.length; i++) {
        const [condition, blockIndex] = block.c[i];
        for (let j = 0; j < condition[1].length; j++) {
          addRoot(roots, condition[1][j], scopes);
        }
        const child = meta.b?.[blockIndex];
        if (child) stack.push({ block: child, scopes });
      }
    }

    if (block.r) {
      for (let i = 0; i < block.r.length; i++) {
        const [collection, itemVar, blockIndex] = block.r[i];
        addRoot(roots, collection, scopes);
        const child = meta.b?.[blockIndex];
        if (child) stack.push({ block: child, scopes: { name: itemVar, parent: scopes } });
      }
    }
  }

  const collected = Array.from(roots);
  templateRootsCache.set(meta, collected);
  return collected;
}

/**
 * Return a cached `Set` view of template roots for fast membership checks.
 *
 * `CoreElement` calls this on SSR state and attribute updates to decide whether
 * a key can live in hidden template state.
 */
export function getTemplateRootSet(meta: TemplateMeta): ReadonlySet<string> {
  let cached = templateRootSetCache.get(meta);
  if (cached) return cached;
  cached = new Set(collectTemplateRoots(meta));
  templateRootSetCache.set(meta, cached);
  return cached;
}

/**
 * Map observed host attribute names back to template state roots.
 *
 * HTML-only components do not declare `@attr`, so this lets their host
 * attributes still update template bindings. Authored members still take
 * precedence in `CoreElement`.
 */
export function getTemplateAttributeMap(meta: TemplateMeta): ReadonlyMap<string, string> {
  let cached = templateAttributeCache.get(meta);
  if (cached) return cached;

  const roots = collectTemplateRoots(meta);
  const attrs = new Map<string, string>();
  for (let i = 0; i < roots.length; i++) {
    attrs.set(toKebabCase(roots[i]), roots[i]);
  }
  templateAttributeCache.set(meta, attrs);
  return attrs;
}

/**
 * Return true when the template requires authored JavaScript event handlers.
 *
 * Auto-elements must never claim these templates because no generated element
 * can provide the developer's handler methods.
 */
export function templateHasEventHandlers(meta: TemplateMeta): boolean {
  const cached = templateEventCache.get(meta);
  if (cached !== undefined) return cached;

  if (meta.re && meta.re.length > 0) {
    templateEventCache.set(meta, true);
    return true;
  }

  const stack: TemplateBlockMeta[] = [meta];
  while (stack.length > 0) {
    const block = stack.pop();
    if (!block) continue;
    if (block.e && block.e.length > 0) {
      templateEventCache.set(meta, true);
      return true;
    }
    const children = (block as TemplateMeta).b;
    if (children) {
      for (let i = 0; i < children.length; i++) stack.push(children[i]);
    }
  }

  templateEventCache.set(meta, false);
  return false;
}

/**
 * Return true when a scriptless template needs a hydrating auto-element.
 *
 * Static HTML-only templates can stay as plain SSR DOM. Templates with dynamic
 * text, attributes, conditionals, or repeats need `CoreElement` so server/router
 * state and host attribute changes can update rendered output.
 */
export function templateNeedsAutoElement(meta: TemplateMeta): boolean {
  if (!meta.ae || templateHasEventHandlers(meta)) return false;
  return collectTemplateRoots(meta).length !== 0;
}
