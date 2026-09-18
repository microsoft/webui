// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type {
  CompiledCondition,
  CompiledRenderMeta,
  TemplateBlockMeta,
  TemplateMeta,
  TemplateSlot,
} from '../template-types.js';
import { createRepeatKeyState, seedHydratedRepeatKeys } from './diff.js';
import { MAX_FRAGMENT_DEPTH, MAX_FRAGMENT_VISITS } from './fragment-work.js';
import { isComponentStyleResourceMarker } from './styles.js';
import type {
  CondBinding,
  RenderBinding,
  RepeatBinding,
  ScopeFrame,
  TemplateInstance,
} from './types.js';
import { bindingArray, EMPTY_BINDINGS, scopeSourceRoot, templateHasTopology } from './types.js';
import { fragmentSourceId } from '../fragment-inputs.js';
import { isRawStartMarker } from './markers.js';

/** Section-local indexes, discarded after SSR hydration finishes wiring. */
export interface SSRIndex {
  elements: Array<Node | undefined>;
  conds: Comment[];
  repeats: Comment[];
  raws: Array<[Comment, Comment]>;
  renders: Comment[];
  start: Comment | null;
  end: Comment | null;
  /** Authored comments only, for parents referenced by compiled comment successors. */
  comments: Array<Comment[] | undefined>;
}

/** Shared host operations; passing the host does not allocate callback adapters. */
export interface HydrationHost {
  $templateElements(meta: TemplateBlockMeta): Array<Node | undefined>;
  $resolveValue(path: string, scope?: ScopeFrame): unknown;
  $hasStateRoot(path: string, scope?: ScopeFrame): boolean;
  /** Charge an enclosing mount operation when hydration is part of one. */
  $visitHydrationInvocation?(depth: number): void;
  /** Resolve caller input and isolated callee scope without mounting or updating. */
  $createHydrationRender(
    owner: TemplateInstance,
    meta: CompiledRenderMeta,
    anchor: Comment,
    end: Comment,
    sourceId?: number,
  ): RenderBinding;
  $wireHydrationSection(instance: TemplateInstance, meta: TemplateBlockMeta, index: SSRIndex): void;
  $hydratedRepeat?(binding: RepeatBinding, items: unknown[], known: boolean): void;
}

interface Shape {
  meta: TemplateBlockMeta;
  elements: Array<Node | undefined>;
  /** 0: opaque component; 1: graph validation; 2: compiled children; 3: outlet. */
  descend: number[];
  commentParents: boolean[];
  raws: TemplateSlot[];
}

let hydrationCursor: typeof hydrateTemplate | undefined;

interface Section extends SSRIndex {
  shape: Shape;
  instance: TemplateInstance;
  present: boolean;
  elementCount: number;
}

interface Cursor {
  parent: Node;
  parentIndex: number;
  next: Node | null;
  section: Section;
}

interface RepeatCursor extends Cursor {
  kind: 'repeat';
  binding: RepeatBinding;
  block: TemplateBlockMeta;
  items: unknown[];
  known: boolean;
}

type Frame =
  | (Cursor & { kind: 'dom' | 'item' })
  | (Cursor & { kind: 'cond'; binding: CondBinding })
  | (Cursor & { kind: 'render'; binding: RenderBinding })
  | RepeatCursor
  | (Cursor & { kind: 'raw'; start: Comment; label: string; endLabel: string; depth: number });

/** Hydrate every compiled section in one iterative walk without relocating SSR nodes. */
export function hydrateTemplate(
  root: Node,
  meta: TemplateMeta,
  host: HydrationHost,
  retainRanges = false,
): TemplateInstance {
  return (hydrationCursor ??= createHydrationCursor())(root, meta, host, retainRanges);
}

function createHydrationCursor(): typeof hydrateTemplate {
  let shapes: WeakMap<TemplateBlockMeta, Shape> | undefined;
  return hydrate;

  function hydrate(
    root: Node,
    meta: TemplateMeta,
    host: HydrationHost,
    retainRanges = false,
  ): TemplateInstance {
    const section = createSection(templateShape(meta, host), root, retainRanges);
    let current: Frame | null = { kind: 'dom', parent: root, parentIndex: 0, next: root.firstChild, section };
    let stack: Frame[] | undefined;
    let sections: Section[] | undefined;
    let visits = 0;
    let rootNodeCount = 0;
    while (current) {
      let frame: Frame = current;
      const node = frame.next;
      if (!node) {
        if (frame.kind !== 'dom') invalid(`missing closing marker for ${frame.kind}`);
        current = stack?.pop() ?? null;
        continue;
      }
      const next = node.nextSibling;
      frame.next = next;
      const type = node.nodeType;
      const data = type === 8 ? (node as Comment).data : '';
      const marker = node as Comment;
      if (frame.kind === 'item' && (data === 'wi' || data === '/wr')) {
        finish(frame.section, marker);
        current = frame = stack!.pop()!;
        frame.next = next;
      }
      const owner = frame.section;
      let own: TemplateInstance | undefined = owner.instance;
      let enter: Frame | null | undefined;
      if (frame.kind === 'raw') {
        if (data === frame.label) frame.depth++;
        else if (data === frame.endLabel && --frame.depth === 0) {
          if (frame.label !== 'wo') owner.raws.push([frame.start, marker]);
          enter = null;
        }
        if (retainRanges && enter !== null) own = undefined;
      } else if (frame.kind === 'repeat') {
        enter = repeatBoundary(frame, marker, data, host);
        if (retainRanges && enter) own = enter.section.instance;
      } else if (type === 8) {
        enter = structuralFrame(frame, marker, data, meta, host, visits);
        if (enter?.kind === 'render') visits++;
        else if (enter === null) own = owner.instance.parent;
      } else {
        enter = staticNode(frame, node, type, retainRanges);
      }
      while (own && frame.parent === own.container && own.nodes !== EMPTY_BINDINGS) {
        if (!retainRanges && !own.parent) own.nodes[rootNodeCount++] = node;
        else own.nodes.push(node);
        if (retainRanges) break;
        own = own.parent;
      }
      if (enter) {
        if (enter.section !== owner) {
          enter.section.start = marker;
          if (retainRanges) enter.section.instance.order = (sections?.length ?? 0) + 1;
          if (sections) sections.push(enter.section);
          else sections = [enter.section];
        }
        if (stack) stack.push(frame);
        else stack = [frame];
        current = enter;
      } else if (enter === null) {
        current = stack!.pop()!;
        current.next = next;
      }
    }
    finish(section, null);
    if (!retainRanges && section.instance.nodes !== EMPTY_BINDINGS) {
      section.instance.nodes.length = rootNodeCount;
    }
    // Nothing is wired (and no marker label is erased) until every range validates.
    host.$wireHydrationSection(section.instance, meta, section);
    if (sections) {
      for (let i = 0; i < sections.length; i++) {
        const entry = sections[i];
        if (entry.present) host.$wireHydrationSection(entry.instance, entry.shape.meta, entry);
      }
    }
    cleanupMarkers(section, retainRanges);
    if (sections) {
      for (let i = 0; i < sections.length; i++) cleanupMarkers(sections[i], retainRanges);
    }
    if (!retainRanges) {
      compact(section.instance);
      if (sections) {
        for (let i = 0; i < sections.length; i++) compact(sections[i].instance);
      }
    }
    return section.instance;
  }

  function repeatBoundary(frame: RepeatCursor, node: Comment, data: string, host: HydrationHost): Frame | null {
    const binding = frame.binding;
    if (data === '/wr') {
      binding.end = node;
      if (frame.known) seedHydratedRepeatKeys(binding, frame.items);
      host.$hydratedRepeat?.(binding, frame.items, frame.known);
      return null;
    }
    if (data !== 'wi') invalid('repeat content is missing an item marker');
    const index = binding.instances.length;
    const scope: ScopeFrame = {
      name: binding.itemVar, value: frame.items[index], parent: binding.scope,
      known: frame.known && index < frame.items.length,
    };
    const retainRanges = binding.owner.range === true;
    if (retainRanges) scope.sourceRoot = scopeSourceRoot(binding.collection, binding.scope);
    const section = createSection(templateShape(frame.block, host), frame.parent, retainRanges, binding.owner, scope);
    binding.instances.push(section.instance);
    return { kind: 'item', parent: frame.parent, parentIndex: 0, next: frame.next, section };
  }

  function structuralFrame(
    frame: Frame,
    node: Comment,
    data: string,
    meta: TemplateMeta,
    host: HydrationHost,
    visits: number,
  ): Frame | null | undefined {
    const owner = frame.section;
    const retainRanges = owner.instance.range === true;
    if (data === 'wo') {
      const index = owner.elementCount;
      const expected = owner.shape.elements[index];
      if (owner.shape.descend[index] !== 3
        || expected?.parentNode !== owner.shape.elements[frame.parentIndex]) {
        invalid('outlet marker does not match its compiled position');
      }
      owner.elements[index] = node;
      owner.elementCount = index + 1;
      return {
        kind: 'raw', parent: frame.parent, parentIndex: frame.parentIndex, next: frame.next,
        section: owner, start: node, label: 'wo', endLabel: '/wo', depth: 1,
      };
    }
    if (data === '/wo') invalid('unpaired outlet end marker');
    if (data === 'wc') {
      const entry = owner.shape.meta.c?.[owner.conds.length];
      if (!entry) invalid('unexpected conditional marker');
      checkSlot(owner, entry[2], frame.parent);
      owner.conds.push(node);
      const binding: CondBinding = {
        condition: entry[0] as CompiledCondition, blockIndex: entry[1],
        anchor: node, scope: owner.instance.scope, owner: owner.instance, instance: null,
      };
      owner.instance.conds.push(binding);
      const section = createSection(templateShape(block(meta, entry[1]), host), frame.parent, retainRanges,
        owner.instance, binding.scope);
      binding.instance = section.instance;
      return { kind: 'cond', parent: frame.parent, parentIndex: 0, next: frame.next, section, binding };
    }
    if (data === 'wf' || data.startsWith('wf:')) {
      const sourceId = data === 'wf' ? undefined : fragmentSourceId(data);
      const entry = owner.shape.meta.u?.[owner.renders.length];
      if (!entry) invalid('unexpected fragment invocation marker');
      if (sourceId !== undefined && entry.length !== 4) invalid('source marker on a parameterless invocation');
      checkSlot(owner, entry[1], frame.parent);
      const depth = (owner.instance.callDepth ?? 0) + 1;
      if (depth > MAX_FRAGMENT_DEPTH) {
        throw new Error(`[WebUI] Fragment hydration exceeds the active invocation depth limit (${MAX_FRAGMENT_DEPTH}).`);
      }
      if (visits >= MAX_FRAGMENT_VISITS) {
        throw new Error(`[WebUI] Fragment hydration exceeds the invocation visit limit (${MAX_FRAGMENT_VISITS}).`);
      }
      host.$visitHydrationInvocation?.(depth);
      owner.renders.push(node);
      const binding = host.$createHydrationRender(owner.instance, entry, node, node, sourceId);
      owner.instance.renders!.push(binding);
      const section = createSection(templateShape(block(meta, entry[0]), host), frame.parent, retainRanges,
        owner.instance, binding.alias);
      section.instance.callDepth = depth;
      binding.instance = section.instance;
      return { kind: 'render', parent: frame.parent, parentIndex: 0, next: frame.next, section, binding };
    }
    if (data === 'wr') {
      const markerId = owner.repeats.length;
      const entry = owner.shape.meta.r?.[markerId];
      if (!entry) invalid('unexpected repeat marker');
      checkSlot(owner, entry[3], frame.parent);
      owner.repeats.push(node);
      const [collection, itemVar, blockIndex, , keyPath] = entry;
      const scope = owner.instance.scope;
      const known = host.$hasStateRoot(collection, scope);
      const value = known ? host.$resolveValue(collection, scope) : undefined;
      const items: unknown[] = Array.isArray(value) ? value : [];
      const binding: RepeatBinding = {
        markerId, collection, itemVar, blockIndex, scope, owner: owner.instance,
        container: frame.parent as ParentNode & Node, start: node, end: null,
        instances: [], synced: known,
      };
      if (keyPath !== undefined) binding.keyState = createRepeatKeyState(keyPath);
      owner.instance.repeats.push(binding);
      return {
        kind: 'repeat', parent: frame.parent, parentIndex: frame.parentIndex, next: frame.next, section: owner, binding,
        block: block(meta, blockIndex), items, known,
      };
    }
    if (data === '/wc' || data === '/wf') {
      if (frame.kind !== 'cond' && frame.kind !== 'render') invalid(`unexpected ${data} marker`);
      if (data !== (frame.kind === 'cond' ? '/wc' : '/wf')) invalid(`unexpected ${data} marker`);
      if (frame.kind === 'render' || retainRanges) frame.binding.end = node;
      if (frame.kind === 'cond' && owner.instance.nodes.length === 0) {
        owner.end = node;
        frame.binding.instance = null;
        owner.present = false;
      } else {
        finish(owner, node);
      }
      return null;
    }
    if (data === 'wi' || data === '/wr') invalid(`unexpected ${data} marker`);
    if (isRawStartMarker(data)) {
      const slot = owner.shape.raws[owner.raws.length];
      if (!slot) invalid('unexpected raw marker');
      checkSlot(owner, slot, frame.parent);
      return {
        kind: 'raw', parent: frame.parent, parentIndex: frame.parentIndex, next: frame.next, section: owner,
        start: node, label: data, endLabel: `/${data}`, depth: 1,
      };
    }
    if (data.charCodeAt(0) === 47 && isRawStartMarker(data.slice(1))) invalid('unpaired raw end marker');
    return staticNode(frame, node, 8, retainRanges);
  }

  function invalid(detail: string): never {
    throw new Error(`[WebUI] Invalid template SSR: ${detail}. Rebuild the server and client templates together.`);
  }

  function staticNode(frame: Cursor, node: Node, type: number, retainRanges: boolean): Frame | undefined {
    const section = frame.section;
    if (type === 1 && isComponentStyleResourceMarker(node as Element)) return;
    if (type === 8 && section.shape.commentParents[frame.parentIndex]) {
      (section.comments[frame.parentIndex] ??= []).push(node as Comment);
    }
    if (type !== 1) return;
    const index = section.elementCount;
    if (!section.shape.elements[index]) invalid('unexpected static element');
    const descend = section.shape.descend[index];
    if (descend === 3) invalid('outlet is missing its range markers');
    section.elements[index] = node;
    section.elementCount = index + 1;
    if (descend === 2 || (descend === 1 && retainRanges)) {
      return { kind: 'dom', parent: node, parentIndex: index, next: node.firstChild, section };
    }
  }

  function templateShape(meta: TemplateBlockMeta, host: HydrationHost): Shape {
    const cached = shapes?.get(meta);
    if (cached) return cached;
    const raws: TemplateSlot[] = [];
    const elements = host.$templateElements(meta);
    const commentParents: boolean[] = [];
    const descend: number[] = [];
    for (const entry of meta.tx ?? []) {
      const parent = entry[0][0];
      if (entry[2] === 1) {
        descend[parent] = 2;
        raws.push(entry[0]);
      } else {
        descend[parent] ??= 1;
        if ((entry[2] ?? 0) % 8 === 7) commentParents[parent] = true;
      }
    }
    for (const entry of meta.c ?? []) descend[entry[2][0]] = 2;
    for (const entry of meta.r ?? []) descend[entry[3][0]] = 2;
    for (const entry of meta.u ?? []) descend[entry[1][0]] = 2;
    for (let i = 1; i < elements.length; i++) {
      const element = elements[i] as Element | undefined;
      if (element?.firstChild) descend[i] = 2;
      else if (descend[i] === undefined) {
        const name = element?.localName;
        descend[i] = name === 'outlet' ? 3 : name && name.indexOf('-') < 0 ? 1 : 0;
      }
    }
    const shape = { meta, elements, descend, commentParents, raws };
    (shapes ??= new WeakMap()).set(meta, shape);
    return shape;
  }

  function createSection(
    shape: Shape,
    container: Node,
    retainRanges: boolean,
    parent?: TemplateInstance,
    scope?: ScopeFrame,
  ): Section {
    const meta = shape.meta;
    const instance: TemplateInstance = {
      scope, container: container as ParentNode & Node,
      nodes: !parent && !retainRanges
        ? templateHasTopology(meta, shape.raws.length)
          ? new Array<Node>(container.childNodes.length)
          : EMPTY_BINDINGS
        : [],
      texts: EMPTY_BINDINGS,
      attrs: bindingArray(meta.a?.length ?? 0),
      conds: bindingArray(meta.c?.length ?? 0),
      repeats: bindingArray(meta.r?.length ?? 0),
    };
    if (parent) instance.parent = parent;
    if (retainRanges) {
      instance.renders = bindingArray(meta.u?.length ?? 0);
      instance.range = true;
      instance.alive = true;
      instance.generation = 0;
      instance.callDepth = parent?.callDepth ?? 0;
      instance.order = 0;
    }
    // The root is present. A conditional section may be absent, so do not
    // reserve its entire static body before seeing any of its elements.
    const elements: Array<Node | undefined> = parent
      ? [container]
      : new Array<Node | undefined>(shape.elements.length);
    elements[0] = container;
    return {
      shape, instance, present: true, elementCount: 1, elements,
      conds: bindingArray(meta.c?.length ?? 0),
      repeats: bindingArray(meta.r?.length ?? 0),
      renders: bindingArray(meta.u?.length ?? 0),
      raws: bindingArray(shape.raws.length),
      start: null, end: null,
      comments: bindingArray(shape.commentParents.length),
    };
  }

  function block(meta: TemplateMeta, index: number): TemplateBlockMeta {
    return meta.b?.[index] ?? invalid(`missing block ${index}`);
  }

  function checkSlot(section: Section, slot: TemplateSlot, parent: Node): void {
    if (section.elements[slot[0]] !== parent) invalid('marker has the wrong section parent');
  }

  function finish(section: Section, end: Comment | null): void {
    const { shape } = section;
    const meta = shape.meta;
    if (
      section.conds.length !== (meta.c?.length ?? 0)
      || section.repeats.length !== (meta.r?.length ?? 0)
      || section.renders.length !== (meta.u?.length ?? 0)
      || section.raws.length !== shape.raws.length
      || section.elementCount !== shape.elements.length
    ) {
      invalid('section nodes do not match compiled metadata');
    }
    section.end = end;
  }

  function cleanupMarkers(entry: Section, retainRanges: boolean): void {
    const conditions = entry.instance.conds;
    for (let i = 0; i < conditions.length; i++) {
      const condition = conditions[i];
      if (retainRanges) {
        condition.anchor!.data = '';
        condition.end!.data = '';
      } else {
        if (condition.instance) {
          condition.anchor?.parentNode?.removeChild(condition.anchor);
          condition.anchor = null;
        }
      }
    }
    const repeats = entry.instance.repeats;
    for (let i = 0; i < repeats.length; i++) {
      const repeat = repeats[i];
      if (retainRanges) {
        repeat.start!.data = '';
        repeat.end!.data = '';
        for (let j = 0; j < repeat.instances.length; j++) {
          (repeat.instances[j].nodes[0] as Comment).data = '';
        }
      } else {
        repeat.end?.parentNode?.removeChild(repeat.end);
        repeat.end = null;
      }
    }
    const renders = entry.instance.renders;
    if (renders) {
      for (let i = 0; i < renders.length; i++) {
        const render = renders[i];
        render.anchor.data = '';
        render.end.data = '';
      }
    }
    if (!retainRanges && entry.start) {
      const start = entry.start;
      const marker = start.data;
      if (marker === 'wi') start.parentNode?.removeChild(start);
      else if (marker === 'wc') entry.end?.parentNode?.removeChild(entry.end);
    }
  }

  function compact(instance: TemplateInstance): void {
    const nodes = instance.nodes;
    if (nodes.length === 0) return;
    let write = 0;
    for (let read = 0; read < nodes.length; read++) {
      const node = nodes[read];
      if (node.parentNode === instance.container) nodes[write++] = node;
    }
    nodes.length = write;
  }
}
