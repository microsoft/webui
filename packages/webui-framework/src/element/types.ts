// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Shared runtime types for the WebUI element system.
 *
 * These types define the binding data structures that are created once during
 * hydration and then read on every reactive update.  Each binding holds a
 * direct DOM node reference so that `$updateInstance` can patch the DOM in
 * O(1) per binding without any tree walking or selector queries.
 *
 * ## Key concepts
 *
 * - **TemplateInstance** — a connected block of DOM with its bindings.  The
 *   root component has one instance; conditionals and repeat items each get
 *   their own nested instance.
 *
 * - **ScopeFrame** — a linked-list frame for repeat variable scoping.
 *   `@for(item of items)` creates a frame `{ name: 'item', value, parent }`.
 *   Nested repeats chain frames so inner bindings can resolve outer variables.
 *
 * - **RepeatHost** — the minimal interface that the repeat module needs from
 *   the host `WebUIElement`, avoiding a circular class dependency.
 */

import type {
  CompiledAttrMeta,
  CompiledAttrPart,
  CompiledConditionExpr,
} from '../template.js';

/** Direct reference to a text node bound to a property path. */
export interface TextBinding {
  node: Text;
  path?: string;
  parts?: CompiledAttrPart[];
  scope?: ScopeFrame;
}

/** Direct reference to an attribute binding with optional static prefix/suffix. */
export interface AttrBinding {
  element: Element;
  name: string;
  kind: 'attribute' | 'complex' | 'boolean' | 'template';
  path?: string;
  condition?: CompiledConditionExpr;
  parts?: CompiledAttrPart[];
  scope?: ScopeFrame;
}

export interface ScopeFrame {
  name: string;
  value: unknown;
  parent?: ScopeFrame;
}

export interface TemplateInstance {
  scope?: ScopeFrame;
  nodes: Node[];
  texts: TextBinding[];
  attrs: AttrBinding[];
  conds: CondBinding[];
  repeats: RepeatBinding[];
}

/** Direct reference to a conditional block with anchor + nested compiled block. */
export interface CondBinding {
  condition: CompiledConditionExpr;
  blockIndex: number;
  anchor: Comment;
  scope?: ScopeFrame;
  instance: TemplateInstance | null;
}

/** Repeat block tracking. */
export interface RepeatBinding {
  markerId: number;
  collection: string;
  itemVar: string;
  blockIndex: number;
  container: (ParentNode & Node) | null;
  start: Comment | null;
  end: Comment | null;
  scope?: ScopeFrame;
  owner: TemplateInstance;
  instances: RepeatItemInstance[];
  rootTag: string | null;
  attrMap: Record<string, string>;
  rootBindings: CompiledAttrMeta[];
  /** Set to true once the collection has been explicitly set by client code. */
  synced?: boolean;
}

export interface RepeatItemInstance {
  key: string | null;
  value: unknown;
  instance: TemplateInstance;
}

export interface ResolvedSlot {
  parent: ParentNode & Node;
  nextSibling: Node | null;
  order: number;
}

/**
 * Minimal host interface for repeat operations.
 *
 * Repeat functions need access to host capabilities (value resolution,
 * block lookup, instance management) without depending on the full
 * WebUIElement class.
 */
export interface RepeatHost {
  $resolveValue(path: string, scope?: ScopeFrame): unknown;
  $block(blockIndex: number): import('../template.js').TemplateBlockMeta | undefined;
  $createBlockInstance(blockIndex: number, scope?: ScopeFrame): TemplateInstance | null;
  $hydrateExistingBlockInstance(blockIndex: number, nodes: Node[], scope?: ScopeFrame): TemplateInstance | null;
  $updateInstance(instance: TemplateInstance): void;
  $removeInstance(instance: TemplateInstance): void;
  $insertInstanceAfter(cursor: Node | null, container: ParentNode & Node, instance: TemplateInstance): Node | null;
  $singleDynamicAttrPart(parts: CompiledAttrPart[]): { path: string; prefix: string; suffix: string } | null;
  $stripAffixes(raw: string, prefix: string, suffix: string): string | undefined;
  $readAttrBindingRange(el: Element): [number, number] | null;
}

export function toCamelCase(str: string): string {
  return str.replace(/-([a-z])/g, (_m, ch: string) => ch.toUpperCase());
}
