// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  finishRender,
  observeRender,
  pendingRenders,
  RENDER,
  RENDER_PENDING,
} from './render-scheduler.js';

const DEPTH = Symbol('render.depth');

interface WorkElement extends Element {
  [DEPTH]?: number;
  [RENDER_PENDING]?: number;
}

interface Completion {
  readonly root: Element;
  readonly pending: Set<WorkElement>;
  readonly observers: Set<Completion>;
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
export let observingRender = false;

const gated = new Map<WorkElement, number>();
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
  let node: Node | null = element;
  while (node) {
    if (node === root) return true;
    node = (node as Element).assignedSlot ?? node.parentNode ??
      (node.nodeType === 11 ? (node as ShadowRoot).host ?? null : null);
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
  observingRender = completions.size !== 0;
  if (!observingRender) observeRender(undefined);
  opening.delete(completion);
  for (const element of completion.pending) {
    const waiters = workByElement.get(element);
    if (waiters?.size === 1) workByElement.delete(element);
    else waiters?.delete(completion);
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
    work = completion.observers;
    workByElement.set(element, work);
  } else if (!work.has(completion)) {
    // A single observer is shared across its jobs; only overlaps need a copy.
    if (work.size === 1) {
      work = new Set(work);
      workByElement.set(element, work);
    }
    work.add(completion);
  }
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
  if (immediate && !observingRender) return false;
  if (observingRender && element.isConnected) expectedRemovals.delete(element);
  gated.set(element, (gated.get(element) ?? 0) | kind);
  if (observingRender) enroll(element);
  return true;
}

function detach(element: WorkElement): void {
  if (!observingRender) return;
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
  const remaining = (gated.get(element) ?? 0) & ~kind;
  if (remaining) gated.set(element, remaining);
  else gated.delete(element);
  if (!remaining && !element[RENDER_PENDING] && !element[DEPTH]) detach(element);
}

export function enterRenderWork(element: WorkElement): Set<Completion> | undefined {
  const previous = active;
  if (!observingRender) {
    if (element[DEPTH]) element[DEPTH]++;
    return previous;
  }
  element[DEPTH] = (element[DEPTH] ?? 0) + 1;
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
  const depth = element[DEPTH];
  if (!depth) return;
  element[DEPTH] = depth - 1;
  if (depth === 1 && !element[RENDER_PENDING] && !gated.has(element)) detach(element);
}

export function invokeRenderCallback(
  element: Element,
  callback: (oldValue: unknown, newValue: unknown) => void,
  oldValue: unknown,
  newValue: unknown,
): void {
  const observed = observingRender;
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
  if (!observingRender) return;
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
  element[DEPTH] = 0;
  gated.delete(element);
  finishRender(element);
  detach(element);
  expectedRemovals.delete(element);
}

export function expectRenderRemoval(
  node: Node,
  end?: Node,
): readonly Node[] | undefined {
  const previous = removing;
  if (!observingRender) return previous;
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
      observers: new Set(),
      resolve,
      reject,
      open: true,
    };
    completion.observers.add(completion);
    completions.add(completion);
    observingRender = true;
    observeRender({
      queued(target) {
        if (target.isConnected) expectedRemovals.delete(target);
        enroll(target);
      },
      render(target) {
        const previous = enterRenderWork(target);
        try {
          target[RENDER]();
        } finally {
          leaveRenderWork(target, previous);
        }
      },
      finished(target) { finishRenderWork(target, 0); },
    });
    opening.add(completion);
    for (const node of pendingRenders) {
      if (node) include(node, completion);
    }
    for (const node of gated.keys()) include(node, completion);
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
