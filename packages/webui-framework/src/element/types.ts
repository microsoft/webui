// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Shared runtime types for the WebUI element system.
 *
 * These types define the binding data structures that are created once during
 * hydration and then read on every reactive update.  Each binding holds a
 * direct DOM node reference so that updates can patch the DOM in O(1) per
 * binding without any tree walking or selector queries.
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
 */

import type {
  CompiledAttrMeta,
  CompiledAttrPart,
  CompiledCondition,
} from '../template.js';

/** Direct reference to a text node bound to a property path. */
export interface TextBinding {
  node: Text;
  path?: string;
  parts?: CompiledAttrPart[];
  scope?: ScopeFrame;
  /** When true, the binding renders unescaped HTML via innerHTML on the
   *  parent element instead of setting Text.data. Corresponds to the
   *  triple-brace `{{{expr}}}` template syntax. */
  raw?: boolean;
  /** The parent element for raw bindings — innerHTML is set here. */
  rawParent?: Element;
}

/** Attribute binding kind constants (matches compiled metadata). */
export const ATTR_KIND_ATTRIBUTE = 0;
export const ATTR_KIND_COMPLEX = 1;
export const ATTR_KIND_BOOLEAN = 2;
export const ATTR_KIND_TEMPLATE = 3;

/** Direct reference to an attribute binding. */
export interface AttrBinding {
  element: Element;
  name: string;
  kind: number;
  path?: string;
  condition?: CompiledCondition;
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
  /**
   * Per-instance listener cleanup. Delegated event listeners attach to the
   * component render root for correctness while detached blocks are moved into
   * place, so nested conditional/repeat instances must explicitly unregister
   * when their block leaves the DOM.
   */
  cleanups?: Array<() => void>;
}

/** Direct reference to a conditional block with anchor + nested compiled block. */
export interface CondBinding {
  condition: CompiledCondition;
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

/**
 * Minimal host interface for repeat operations.
 *
 * Repeat functions need access to host capabilities (value resolution,
 * block lookup, instance management) without depending on the full
 * WebUIElement class.
 */
export interface RepeatHost {
  $resolveValue(path: string, scope?: ScopeFrame): unknown;
  /** Create, wire, and perform the first binding pass while detached. */
  $createBlockInstance(blockIndex: number, scope?: ScopeFrame): TemplateInstance | null;
  $updateInstance(instance: TemplateInstance): void;
  $removeInstance(instance: TemplateInstance): void;
  $insertInstanceAfter(cursor: Node | null, container: ParentNode & Node, instance: TemplateInstance): Node | null;
}
