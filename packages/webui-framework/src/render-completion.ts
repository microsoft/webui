// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  enqueueRender,
  finishRender,
  observeRender,
  pendingRenders,
  RENDER,
  RENDER_PENDING,
  type RenderTarget,
} from './render-scheduler.js';

interface WorkElement extends Element {
  [RENDER_PENDING]?: number;
}

type Work = WorkElement | RenderTask;

interface Completion {
  readonly root: Element;
  readonly jobs: Set<Work>;
  readonly group: Set<Completion>;
  readonly resolve: () => void;
  readonly reject: (error: unknown) => void;
  tasks: number;
  blockers?: Set<BlockedTarget>;
  open: boolean;
}

export interface RenderTask {
  readonly element: Element;
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
const blockedByElement = new WeakMap<Element, BlockedTarget>();
const blocked = new Set<BlockedTarget>();
let pendingTasks: Map<Element, Set<RenderTask>> | undefined;
let retryTasks: ((root: Element) => void) | undefined;
const finalizedBlocked = new FinalizationRegistry<BlockedTarget>(
  reference => { blocked.delete(reference); },
);
type RenderObserverState = ReturnType<typeof createRenderObserver>;
let renderState: RenderObserverState | undefined;
export { renderState as renderWork };
let cachedState: WeakRef<RenderObserverState> | undefined;
let retiring = false;

function retireRenderObserver(): void {
  retiring = false;
  if (!observingRender) renderState = undefined;
}

export function inScope(element: Element, root: Node): boolean {
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

function unqueueTask(task: RenderTask): void {
  const tasks = pendingTasks?.get(task.element);
  if (!tasks?.delete(task)) return;
  if (!tasks.size) pendingTasks?.delete(task.element);
}

export function installRenderTaskRetry(retry: (root: Element) => void): void {
  retryTasks = retry;
}

export function unblockRenderWork(element: Element): void {
  const state = blockedByElement.get(element);
  if (!state) return;
  blockedByElement.delete(element);
  blocked.delete(state);
  finalizedBlocked.unregister(state);
}

export function queueRenderTask(task: RenderTask): void {
  const pending = pendingTasks ??= new Map();
  let tasks = pending.get(task.element);
  if (!tasks) pending.set(task.element, tasks = new Set());
  tasks.add(task);
  if (observingRender) renderState?.enroll(task.element, task);
}

export function startRenderTask(element: Element): RenderTask {
  const task: RenderTask = { element };
  queueRenderTask(task);
  return task;
}

export function enterRenderTask(task: RenderTask): Set<Completion> | undefined {
  const state = renderState;
  if (!state) return undefined;
  const previous = state.active;
  if (!observingRender || !pendingTasks?.get(task.element)?.has(task)) return previous;
  const observers = state.observers.get(task);
  if (observers) {
    for (const completion of observers) {
      if (!task.element.isConnected || !inScope(task.element, completion.root)) {
        state.abort(completion);
      }
    }
  }
  state.active = state.observers.get(task);
  return previous;
}

export function leaveRenderTask(previous: Set<Completion> | undefined): void {
  const state = renderState;
  if (!state) return undefined;
  state.active = previous;
}

export function associateRenderTask(task: RenderTask, element: WorkElement): boolean {
  const state = renderState;
  if (!state) return false;
  if (!observingRender) return false;
  const observers = state.observers.get(task);
  if (!observers) return false;
  let associated = false;
  for (const completion of observers) {
    state.include(element, completion);
    if (completion.jobs.has(element)) associated = true;
  }
  return associated;
}

export function finishRenderTask(task: RenderTask, error?: { readonly value: unknown }): void {
  unqueueTask(task);
  const state = renderState;
  if (!state) return undefined;
  const observers = state.observers.get(task);
  if (!observers) return;
  state.observers.delete(task);
  for (const completion of observers) {
    completion.jobs.delete(task);
    completion.tasks--;
    if (error) state.release(completion, error);
    else state.settle(completion);
  }
}

export function cancelRenderTask(task: RenderTask): void {
  unqueueTask(task);
  const state = renderState;
  if (!state) return undefined;
  const observers = state.observers.get(task);
  if (!observers) return;
  state.observers.delete(task);
  for (const completion of observers) {
    completion.jobs.delete(task);
    completion.tasks--;
    if (state.active?.has(completion)) state.settle(completion);
    else state.abort(completion);
  }
}

export function queueRenderWork(element: WorkElement, kind: number, immediate = false): boolean {
  if (immediate && !observingRender) return false;
  if (renderState) renderState.queued(element, kind);
  else queueUnobservedWork(element, kind);
  return true;
}

function queueUnobservedWork(element: WorkElement, kind?: number): void {
  if (kind !== undefined) gated.set(element, (gated.get(element) ?? 0) | kind);
}

export function finishRenderWork(element: WorkElement, kind: number): void {
  const remaining = (gated.get(element) ?? 0) & ~kind;
  if (remaining) gated.set(element, remaining);
  else gated.delete(element);
  renderState?.finished(element);
}

export function invokeRenderCallback(
  element: Element, callback: (oldValue: unknown, newValue: unknown) => void,
  oldValue?: unknown, newValue?: unknown,
): void {
  const observed = observingRender;
  const previous = observed ? renderState?.enter(element) : undefined;
  try {
    callback.call(element, oldValue, newValue);
  } catch (error) {
    renderState?.fail(element, error);
    throw error;
  } finally {
    if (observed) renderState?.leave(element, previous);
  }
}

export function blockRenderWork(element: Element, reason: string): void {
  let state = blockedByElement.get(element);
  if (state) state.reason = reason;
  else {
    state = new BlockedTarget(element, reason);
    blockedByElement.set(element, state);
    blocked.add(state);
    finalizedBlocked.register(element, state, state);
  }
  if (observingRender) renderState?.blocked(element, state);
}

export function destroyRenderWork(element: WorkElement): void {
  unblockRenderWork(element);
  const tasks = pendingTasks?.get(element);
  if (tasks) for (const task of tasks) cancelRenderTask(task);
  if (renderState) renderState.destroy(element);
  else destroyUnobservedWork(element);
}

function destroyUnobservedWork(element: WorkElement): void {
  gated.delete(element);
  finishRender(element);
}

export function whenRendered(element: Element, unavailable?: string): Promise<void> {
  if (unavailable) return Promise.reject(invalidState(element, unavailable));
  if (!renderState) {
    renderState = cachedState?.deref();
    if (!renderState) {
      renderState = createRenderObserver();
      cachedState = new WeakRef(renderState);
    }
  }
  return renderState.complete(element);
}

function createRenderObserver() {
  let active: Set<Completion> | undefined;
  const completions = new Set<Completion>();
  const opening = new Set<Completion>();
  const observersByWork = new WeakMap<Work, Set<Completion>>();
  const executing = new WeakMap<WorkElement, number>();
  const expectedRemovals = new WeakMap<Work, Set<Completion>>();
  let removing: readonly Node[] | undefined;

  function release(completion: Completion, error?: { readonly value: unknown }): void {
    if (!completions.delete(completion)) return;
    observingRender = completions.size !== 0;
    if (!observingRender) {
      observeRender(undefined);
      if (!retiring) {
        retiring = true;
        queueMicrotask(retireRenderObserver);
      }
    }
    opening.delete(completion);
    for (const work of completion.jobs) {
      const waiters = observersByWork.get(work);
      if (waiters?.size === 1) observersByWork.delete(work);
      else waiters?.delete(completion);
      const expected = expectedRemovals.get(work);
      if (expected?.delete(completion) && !expected.size) {
        expectedRemovals.delete(work);
      }
    }
    completion.jobs.clear();
    completion.blockers?.clear();
    if (error) completion.reject(error.value);
    else completion.resolve();
  }

  function settle(completion: Completion): void {
    if (completion.open || completion.jobs.size) return;
    if (completion.blockers) {
      for (const reference of completion.blockers) {
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

  function include(element: WorkElement, completion: Completion, work: Work = element): void {
    if (completion.jobs.has(work) ||
        !completions.has(completion) || !inScope(element, completion.root)) return;
    let observers = observersByWork.get(work);
    if (!observers) {
      observers = completion.group;
      observersByWork.set(work, observers);
    } else if (!observers.has(completion)) {
      // A single observer is shared across its jobs; only overlaps need a copy.
      if (observers.size === 1) {
        observers = new Set(observers);
        observersByWork.set(work, observers);
      }
      observers.add(completion);
    }
    completion.jobs.add(work);
    if (work !== element) completion.tasks++;
  }

  function enroll(element: WorkElement, work?: Work): void {
    const current = active;
    if (current) {
      for (const completion of current) include(element, completion, work);
    }
    for (const completion of opening) {
      if (!current?.has(completion)) include(element, completion, work);
    }
  }

  function abortRenderScope(
    completion: Completion,
    reason = 'Captured render work left its component scope.',
  ): void {
    release(completion, { value: new DOMException(
      '[WebUI] ' + reason,
      'AbortError',
    ) });
  }

  function blockCompletion(
    completion: Completion,
    state: BlockedTarget,
    element: Element,
  ): void {
    if (completion.tasks) {
      (completion.blockers ??= new Set()).add(state);
    } else {
      release(completion, { value: invalidState(element, state.reason) });
    }
  }

  function detach(element: WorkElement): void {
    if (!observingRender) return;
    const work = observersByWork.get(element);
    const expected = expectedRemovals.get(element);
    expectedRemovals.delete(element);
    if (!work) return;
    observersByWork.delete(element);
    for (const completion of work) {
      completion.jobs.delete(element);
      if ((!element.isConnected || !inScope(element, completion.root)) &&
          !expected?.has(completion)) {
        abortRenderScope(completion);
      } else {
        settle(completion);
      }
    }
  }

  function enterRenderWork(
    element: WorkElement,
    queued = false,
  ): Set<Completion> | undefined {
    const previous = active;
    const depth = executing.get(element) ?? 0;
    if (!observingRender) {
      if (depth) executing.set(element, depth + 1);
      return previous;
    }
    executing.set(element, depth + 1);
    try {
      const work = observersByWork.get(element);
      if (work) {
        for (const completion of work) {
          if (!element.isConnected || !inScope(element, completion.root)) {
            if (!expectedRemovals.get(element)?.has(completion)) {
              abortRenderScope(completion);
            }
          }
        }
      }
      // Queued jobs enrolled on writes and initial boundary discovery already.
      if (!queued) enroll(element);
      active = observersByWork.get(element);
      return previous;
    } catch (error) {
      unwindRenderWork(element);
      const work = observersByWork.get(element);
      if (work) {
        for (const completion of work) release(completion, { value: error });
      }
      throw error;
    }
  }

  function unwindRenderWork(element: WorkElement): boolean {
    const depth = executing.get(element);
    if (!depth) return false;
    if (depth > 1) {
      executing.set(element, depth - 1);
      return false;
    }
    executing.delete(element);
    return true;
  }

  function leaveRenderWork(
    element: WorkElement,
    previous: Set<Completion> | undefined,
  ): void {
    active = previous;
    if (unwindRenderWork(element) && !element[RENDER_PENDING] && !gated.has(element)) {
      detach(element);
    }
  }

  function failRenderWork(element: Element, error: unknown): void {
    const work = observersByWork.get(element);
    if (work) {
      for (const completion of work) release(completion, { value: error });
    }
    for (const completion of opening) {
      if (inScope(element, completion.root)) release(completion, { value: error });
    }
  }

  function disconnectRenderWork(element: Element): void {
    for (const completion of completions) {
      if (completion.root === element) {
        abortRenderScope(completion, 'The render-completion root disconnected.');
      }
    }
    const work = observersByWork.get(element);
    const tasks = pendingTasks?.get(element);
    if ((work || tasks) && active && removing) {
      let expectedTarget = false;
      for (const root of removing) {
        if (inScope(element, root)) {
          expectedTarget = true;
          break;
        }
      }
      if (!expectedTarget) return;
      if (work) {
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
      if (tasks) {
        for (const task of tasks) {
          const observers = observersByWork.get(task);
          if (!observers) continue;
          for (const completion of observers) {
            if (!active.has(completion)) continue;
            if (observers.size === 1) observersByWork.delete(task);
            else observers.delete(completion);
            completion.jobs.delete(task);
            completion.tasks--;
            settle(completion);
          }
        }
      }
    }
  }

  function destroyRenderWork(element: WorkElement): void {
    const work = observersByWork.get(element);
    if (work) {
      for (const completion of work) {
        if (!expectedRemovals.get(element)?.has(completion)) {
          abortRenderScope(completion, 'Captured render work was destroyed before completion.');
        }
      }
    }
    executing.delete(element);
    destroyUnobservedWork(element);
    detach(element);
    expectedRemovals.delete(element);
  }

  function expectRenderRemoval(
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

  function finishRenderRemoval(previous: readonly Node[] | undefined): void {
    removing = previous;
  }

  function whenRendered(element: Element): Promise<void> {
    let completion!: Completion;
    const promise = new Promise<void>((resolve, reject) => {
      completion = {
        root: element,
        jobs: new Set(),
        group: new Set(),
        resolve,
        reject,
        tasks: 0,
        open: true,
      };
    });
    completion.group.add(completion);
    completions.add(completion);
    observingRender = true;
    observeRender(renderObserver);
    opening.add(completion);
    try {
      retryTasks?.(element);
      for (const node of pendingRenders) {
        if (node) include(node, completion);
      }
      for (const node of gated.keys()) include(node, completion);
      if (pendingTasks) {
        for (const tasks of pendingTasks.values()) {
          for (const task of tasks) include(task.element, completion, task);
        }
      }
      for (const reference of blocked) {
        const target = reference.deref();
        if (!target) {
          blocked.delete(reference);
          continue;
        }
        if (inScope(target, element)) {
          blockCompletion(completion, reference, target);
          if (!completions.has(completion)) return promise;
        }
      }
      queueMicrotask(() => {
        completion.open = false;
        opening.delete(completion);
        settle(completion);
      });
    } catch (error) {
      release(completion, { value: error });
    }
    return promise;
  }

  const renderObserver = {
    queued(target: Element, kind?: number): void {
      if (observingRender && target.isConnected) expectedRemovals.delete(target);
      queueUnobservedWork(target, kind);
      if (observingRender) enroll(target);
    },
    render(target: RenderTarget): void {
      let previous: Set<Completion> | undefined;
      try {
        previous = enterRenderWork(target, true);
      } catch (error) {
        finishRender(target);
        enqueueRender(target);
        throw error;
      }
      try {
        target[RENDER]();
      } finally {
        leaveRenderWork(target, previous);
      }
    },
    finished(target: WorkElement): void {
      if (!executing.has(target) && !gated.has(target)) detach(target);
    },
    enroll, include, release, settle, abort: abortRenderScope, observers: observersByWork,
    get active() { return active; },
    set active(previous: Set<Completion> | undefined) { active = previous; },
    enter: enterRenderWork, leave: leaveRenderWork, fail: failRenderWork,
    disconnect: disconnectRenderWork, destroy: destroyRenderWork,
    remove: expectRenderRemoval, restoreRemoval: finishRenderRemoval, complete: whenRendered,
    blocked: observeBlocked,
  };

  function observeBlocked(element: Element, state: BlockedTarget): void {
    for (const completion of opening) {
      if (inScope(element, completion.root)) blockCompletion(completion, state, element);
    }
    if (active) {
      for (const completion of active) {
        if (inScope(element, completion.root)) blockCompletion(completion, state, element);
        else (completion.blockers ??= new Set()).add(state);
      }
    }
  }

  return renderObserver;
}
