// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

const PREVIOUS = Symbol('render.previous');
const NEXT = Symbol('render.next');
const FLAGS = Symbol('render.flags');
const RENDER_DEPTH = 4;

interface WorkElement extends Element {
  [PREVIOUS]?: WorkElement | null;
  [NEXT]?: WorkElement;
  [FLAGS]?: number;
}

interface Completion {
  readonly root: Element;
  readonly pending: Set<WorkElement>;
  readonly resolve: () => void;
  readonly reject: (error: unknown) => void;
  stagedBlocked?: Set<BlockedTarget>;
  open: boolean;
}

class BlockedTarget extends WeakRef<Element> {
  constructor(element: Element, public reason: string) {
    super(element);
  }
}

export const UPDATE_WORK = 1;
export const MOUNT_WORK = 2;

let head: WorkElement | undefined;
let active: Set<Completion> | undefined;
const completions = new Set<Completion>();
const opening = new Set<Completion>();
const workByElement = new WeakMap<Element, Set<Completion>>();
const expectedRemovals = new WeakMap<Element, Set<Completion>>();
let removing: readonly Node[] | undefined;
const blockedByElement = new WeakMap<Element, BlockedTarget>();
const blocked = new Set<BlockedTarget>();
const finalizedBlocked = new FinalizationRegistry<BlockedTarget>(
  reference => { blocked.delete(reference); },
);

function inScope(element: Element, root: Node): boolean {
  let node: Element | null = element;
  while (node) {
    if (node === root) return true;
    const parent: Element | null = node.assignedSlot ?? node.parentElement;
    if (parent) {
      node = parent;
    } else {
      const tree = node.getRootNode();
      node = tree.nodeType === 11
        ? (tree as ShadowRoot).host ?? null
        : null;
    }
  }
  return false;
}

function invalidState(element: Element, reason: string): DOMException {
  return new DOMException(
    `[WebUI] Cannot complete rendering for <${element.localName}>: ${reason}. ` +
    'Wait for component readiness before requesting render completion.',
    'InvalidStateError',
  );
}

function release(completion: Completion, error?: { readonly value: unknown }): void {
  if (!completions.delete(completion)) return;
  opening.delete(completion);
  for (const element of completion.pending) {
    const waiters = workByElement.get(element);
    waiters?.delete(completion);
    if (!waiters?.size) workByElement.delete(element);
    const expected = expectedRemovals.get(element);
    if (expected?.delete(completion) && !expected.size) {
      expectedRemovals.delete(element);
    }
  }
  completion.pending.clear();
  completion.stagedBlocked?.clear();
  if (error) completion.reject(error.value);
  else completion.resolve();
}

function settle(completion: Completion): void {
  if (completion.open || completion.pending.size) return;
  if (completion.stagedBlocked) {
    for (const reference of completion.stagedBlocked) {
      const element = reference.deref();
      const state = element && blockedByElement.get(element);
      if (element && state === reference && inScope(element, completion.root)) {
        release(completion, { value: invalidState(element, state.reason) });
        return;
      }
    }
  }
  release(completion);
}

function include(element: WorkElement, completion: Completion): void {
  if (completion.pending.has(element) ||
      !completions.has(completion) || !inScope(element, completion.root)) return;
  let work = workByElement.get(element);
  if (!work) {
    work = new Set();
    workByElement.set(element, work);
  }
  work.add(completion);
  completion.pending.add(element);
}

function enroll(element: WorkElement): void {
  if (active) {
    for (const completion of active) include(element, completion);
  }
  for (const completion of opening) include(element, completion);
}

export function queueRenderWork(
  element: WorkElement,
  kind: number,
  immediate = false,
): boolean {
  if (immediate && !completions.size) return false;
  if (completions.size && element.isConnected) expectedRemovals.delete(element);
  element[FLAGS] = (element[FLAGS] ?? 0) | kind;
  if (element[PREVIOUS] === undefined) {
    element[PREVIOUS] = null;
    element[NEXT] = head;
    if (head) head[PREVIOUS] = element;
    head = element;
  }
  if (completions.size) enroll(element);
  return true;
}

function detach(element: WorkElement): void {
  const previous = element[PREVIOUS];
  if (previous !== undefined) {
    const next = element[NEXT];
    if (previous) previous[NEXT] = next;
    else head = next;
    if (next) next[PREVIOUS] = previous;
    element[PREVIOUS] = undefined;
    element[NEXT] = undefined;
  }
  if (!completions.size) return;
  const work = workByElement.get(element);
  const expected = expectedRemovals.get(element);
  expectedRemovals.delete(element);
  if (!work) return;
  workByElement.delete(element);
  for (const completion of work) {
    completion.pending.delete(element);
    if ((!element.isConnected || !inScope(element, completion.root)) &&
        !expected?.has(completion)) {
      release(completion, { value: new DOMException(
        '[WebUI] Captured render work left its component scope.',
        'AbortError',
      ) });
    } else {
      settle(completion);
    }
  }
}

export function finishRenderWork(element: WorkElement, kind: number): void {
  element[FLAGS] = (element[FLAGS] ?? 0) & ~kind;
  if (!element[FLAGS]) detach(element);
}

export function enterRenderWork(element: WorkElement): Set<Completion> | undefined {
  const previous = active;
  if (!completions.size) {
    const flags = element[FLAGS] ?? 0;
    if (flags >= RENDER_DEPTH) element[FLAGS] = flags + RENDER_DEPTH;
    return previous;
  }
  element[FLAGS] = (element[FLAGS] ?? 0) + RENDER_DEPTH;
  enroll(element);
  const work = workByElement.get(element);
  active = work;
  if (work) {
    for (const completion of work) {
      if (!element.isConnected || !inScope(element, completion.root)) {
        if (!expectedRemovals.get(element)?.has(completion)) {
          release(completion, { value: new DOMException(
            '[WebUI] Captured render work left its component scope.',
            'AbortError',
          ) });
        }
      }
    }
  }
  return previous;
}

export function leaveRenderWork(
  element: WorkElement,
  previous: Set<Completion> | undefined,
): void {
  active = previous;
  const flags = element[FLAGS] ?? 0;
  if (flags < RENDER_DEPTH) return;
  element[FLAGS] = flags - RENDER_DEPTH;
  if (!element[FLAGS]) detach(element);
}

export function invokeRenderCallback(
  element: Element,
  callback: (oldValue: unknown, newValue: unknown) => void,
  oldValue: unknown,
  newValue: unknown,
): void {
  const observed = completions.size !== 0;
  const previous = observed ? enterRenderWork(element) : undefined;
  try {
    callback.call(element, oldValue, newValue);
  } catch (error) {
    failRenderWork(element, error);
    throw error;
  } finally {
    if (observed) leaveRenderWork(element, previous);
  }
}

export function failRenderWork(element: Element, error: unknown): void {
  const work = workByElement.get(element);
  if (work) {
    for (const completion of work) release(completion, { value: error });
  }
  for (const completion of opening) {
    if (inScope(element, completion.root)) release(completion, { value: error });
  }
}

export function blockRenderWork(element: Element, reason: string): void {
  let state = blockedByElement.get(element);
  if (state) {
    state.reason = reason;
  } else {
    state = new BlockedTarget(element, reason);
    blockedByElement.set(element, state);
    blocked.add(state);
    finalizedBlocked.register(element, state, state);
  }
  if (!completions.size) return;
  for (const completion of opening) {
    if (inScope(element, completion.root)) {
      release(completion, { value: invalidState(element, reason) });
    }
  }
  if (active) {
    for (const completion of active) {
      if (inScope(element, completion.root)) {
        release(completion, { value: invalidState(element, reason) });
      } else {
        (completion.stagedBlocked ??= new Set()).add(state);
      }
    }
  }
}

export function unblockRenderWork(element: Element): void {
  const state = blockedByElement.get(element);
  if (!state) return;
  blockedByElement.delete(element);
  blocked.delete(state);
  finalizedBlocked.unregister(state);
}

export function disconnectRenderWork(element: Element): void {
  for (const completion of completions) {
    if (completion.root === element) {
      release(completion, { value: new DOMException(
        '[WebUI] The render-completion root disconnected.',
        'AbortError',
      ) });
    }
  }
  const work = workByElement.get(element);
  if (work && active && removing) {
    let expectedTarget = false;
    for (const root of removing) {
      if (inScope(element, root)) {
        expectedTarget = true;
        break;
      }
    }
    if (!expectedTarget) return;
    for (const completion of work) {
      if (active.has(completion)) {
        let expected = expectedRemovals.get(element);
        if (!expected) {
          expected = new Set();
          expectedRemovals.set(element, expected);
        }
        expected.add(completion);
      }
    }
  }
}

export function destroyRenderWork(element: WorkElement): void {
  unblockRenderWork(element);
  const work = workByElement.get(element);
  if (work) {
    for (const completion of work) {
      if (!expectedRemovals.get(element)?.has(completion)) {
        release(completion, { value: new DOMException(
          '[WebUI] Captured render work was destroyed before completion.',
          'AbortError',
        ) });
      }
    }
  }
  element[FLAGS] = 0;
  detach(element);
  expectedRemovals.delete(element);
}

export function expectRenderRemoval(
  node: Node,
  end?: Node,
): readonly Node[] | undefined {
  const previous = removing;
  if (!completions.size) return previous;
  if (end) {
    const nodes: Node[] = [];
    for (let current = node.nextSibling; current && current !== end; current = current.nextSibling) {
      nodes.push(current);
    }
    removing = nodes;
  } else {
    removing = [node];
  }
  return previous;
}

export function finishRenderRemoval(previous: readonly Node[] | undefined): void {
  removing = previous;
}

export function whenRendered(element: Element, unavailable?: string): Promise<void> {
  if (unavailable) return Promise.reject(invalidState(element, unavailable));
  return new Promise<void>((resolve, reject) => {
    const completion: Completion = {
      root: element,
      pending: new Set(),
      resolve,
      reject,
      open: true,
    };
    completions.add(completion);
    opening.add(completion);
    let node = head;
    while (node) {
      include(node, completion);
      node = node[NEXT];
    }
    for (const reference of blocked) {
      const target = reference.deref();
      if (!target) {
        blocked.delete(reference);
        continue;
      }
      if (inScope(target, element)) {
        release(completion, { value: invalidState(target, reference.reason) });
        return;
      }
    }
    queueMicrotask(() => {
      completion.open = false;
      opening.delete(completion);
      settle(completion);
    });
  });
}
