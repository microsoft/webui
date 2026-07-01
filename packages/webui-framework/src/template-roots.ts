// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

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

function pathRoot(path: string): string {
  const dot = path.indexOf('.');
  return dot === -1 ? path : path.slice(0, dot);
}

function isScopedPath(path: string, scopes?: ScopeName): boolean {
  const root = pathRoot(path);
  let current = scopes;
  while (current) {
    if (current.name === root) return true;
    current = current.parent;
  }
  return false;
}

function addRoot(roots: Set<string>, path: string, scopes?: ScopeName): void {
  if (!path || isScopedPath(path, scopes)) return;
  roots.add(pathRoot(path));
}

function addPartRoots(roots: Set<string>, parts: CompiledAttrPart[], scopes?: ScopeName): void {
  for (let i = 0; i < parts.length; i++) {
    const part = parts[i];
    if (typeof part !== 'string') addRoot(roots, part[0], scopes);
  }
}

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

export function getTemplateRootSet(meta: TemplateMeta): ReadonlySet<string> {
  let cached = templateRootSetCache.get(meta);
  if (cached) return cached;
  cached = new Set(collectTemplateRoots(meta));
  templateRootSetCache.set(meta, cached);
  return cached;
}

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
