// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Shared component-level lazy hydration coordinator.
 *
 * A single viewport observer serves every opted-in WebUIElement. Interaction
 * capture closes the asynchronous-observer race, while a bounded drain keeps a
 * large visible batch from monopolizing the main thread.
 */

import {
  hydrationEnd,
  hydrationStart,
  isHydrationStartupPending,
} from './lifecycle.js';

export const LAZY_HYDRATION_ACTIVATE = Symbol();

type LazyHydrationTarget = HTMLElement & {
  [LAZY_HYDRATION_ACTIVATE](
    state: Record<string, unknown> | undefined,
  ): void;
};

const pendingTargets = new WeakSet<LazyHydrationTarget>();
const observedTargets = new Set<LazyHydrationTarget>();
const boundaryStates = new WeakMap<
  LazyHydrationTarget,
  Record<string, unknown> | undefined
>();
const streamedActivations = new WeakSet<LazyHydrationTarget>();
const initialObservationTargets = new WeakSet<LazyHydrationTarget>();
const activationGenerations = new WeakMap<LazyHydrationTarget, number>();
const queuedGenerations = new WeakMap<LazyHydrationTarget, number>();
const observationStartTimes = new WeakMap<LazyHydrationTarget, number>();
const activationQueue: LazyHydrationTarget[] = [];
const activationQueueGenerations: number[] = [];
const ancestorScratch: LazyHydrationTarget[] = [];
const WAKE_EVENTS = ['pointerdown', 'focus', 'keydown', 'click'] as const;
const HYDRATION_BUDGET_MS = 8;

let observer: IntersectionObserver | undefined;
let queueIndex = 0;
let continuationPending = false;
let schedulerUnavailable = false;
let wakeListenersInstalled = false;
let continuationChannel: MessageChannel | undefined;
let hydrationBatchActive = false;
let initialObservationCount = 0;
let startupObservationGateActive = false;
let startupObservationGateSealed = false;

/** Whether this browser can defer hydration without risking an inert component. */
export function supportsLazyHydration(): boolean {
  return typeof IntersectionObserver !== 'undefined';
}

/** Register an ordinary SSR component for viewport and interaction activation. */
export function observeLazyHydration(target: LazyHydrationTarget): void {
  pendingTargets.add(target);
  if (!target.isConnected || observedTargets.has(target)) return;
  observedTargets.add(target);
  advanceActivationGeneration(target);
  observationStartTimes.set(target, performance.now());
  trackInitialObservation(target);
  getObserver().observe(target);
  installWakeListeners();
}

/**
 * Retain boundary-local state and register a streamed SSR component only after
 * its boundary commits.
 */
export function observeStreamedLazyHydration(
  target: LazyHydrationTarget,
  state: Record<string, unknown> | undefined,
): void {
  boundaryStates.set(target, state);
  observeLazyHydration(target);
}

/** Stop observing a disconnected component without losing reconnect eligibility. */
export function disconnectLazyHydration(target: LazyHydrationTarget): void {
  advanceActivationGeneration(target);
  settleInitialObservation(target);
  if (!observedTargets.delete(target)) return;
  observer?.unobserve(target);
  removeWakeListenersWhenIdle();
}

/** True while a retained streaming activation is entering the normal hydrator. */
export function isStreamedLazyActivation(target: LazyHydrationTarget): boolean {
  return streamedActivations.has(target);
}

function getObserver(): IntersectionObserver {
  if (!observer) {
    const supportsScrollMargin = 'scrollMargin' in IntersectionObserver.prototype;
    const options: IntersectionObserverInit & { scrollMargin?: string } = {
      root: null,
      rootMargin: supportsScrollMargin ? '0px' : '200px',
      threshold: 0,
    };
    if (supportsScrollMargin) options.scrollMargin = '200px';
    observer = new IntersectionObserver(handleIntersections, options);
  }
  return observer;
}

function handleIntersections(entries: IntersectionObserverEntry[]): void {
  if (entries.length === 0) return;
  let queued = false;
  for (let i = 0; i < entries.length; i++) {
    const entry = entries[i];
    if (
      isCurrentObservation(entry) &&
      (entry.isIntersecting || entry.intersectionRatio > 0)
    ) {
      queued = true;
    }
  }
  if (queued) beginHydrationBatch();
  for (let i = 0; i < entries.length; i++) {
    const entry = entries[i];
    const target = entry.target as LazyHydrationTarget;
    if (!isCurrentObservation(entry)) continue;
    settleInitialObservation(target);
    if (entry.isIntersecting || entry.intersectionRatio > 0) {
      enqueueActivationChain(target);
    }
  }
  if (!queued) return;
  if (!continuationPending) drainActivationQueue();
}

function drainActivationQueue(): void {
  continuationPending = false;
  const started = performance.now();
  while (queueIndex < activationQueue.length) {
    const target = activationQueue[queueIndex];
    const generation = activationQueueGenerations[queueIndex];
    queueIndex++;
    if (queuedGenerations.get(target) === generation) {
      queuedGenerations.delete(target);
    }
    if (activationGenerations.get(target) !== generation) continue;
    try {
      activate(target);
    } catch (error) {
      reportActivationError(error);
    }
    if (
      queueIndex < activationQueue.length &&
      performance.now() - started >= HYDRATION_BUDGET_MS
    ) {
      scheduleContinuation();
      return;
    }
  }
  activationQueue.length = 0;
  activationQueueGenerations.length = 0;
  queueIndex = 0;
  finishHydrationBatch();
}

function beginHydrationBatch(): boolean {
  if (hydrationBatchActive) return false;
  hydrationBatchActive = true;
  hydrationStart();
  return true;
}

function finishHydrationBatch(): void {
  if (!hydrationBatchActive) return;
  hydrationBatchActive = false;
  hydrationEnd();
}

function scheduleContinuation(): void {
  if (continuationPending) return;
  continuationPending = true;
  const taskScheduler = (
    globalThis as typeof globalThis & {
      scheduler?: {
        postTask(
          callback: () => void,
          options: { priority: 'user-visible' },
        ): Promise<unknown>;
      };
    }
  ).scheduler;
  if (!schedulerUnavailable && taskScheduler?.postTask) {
    void taskScheduler
      .postTask(drainActivationQueue, { priority: 'user-visible' })
      .catch(abortContinuation);
    return;
  }
  if (!continuationChannel) {
    continuationChannel = new MessageChannel();
    continuationChannel.port1.onmessage = drainActivationQueue;
  }
  continuationChannel.port2.postMessage(0);
}

function abortContinuation(error: unknown): void {
  continuationPending = false;
  schedulerUnavailable = true;
  reportActivationError(error);
  if (queueIndex < activationQueue.length) {
    scheduleContinuation();
  } else {
    finishHydrationBatch();
  }
}

function reportActivationError(error: unknown): void {
  const target = globalThis as typeof globalThis & {
    reportError?: (reported: unknown) => void;
  };
  if (typeof target.reportError === 'function') {
    target.reportError(error);
    return;
  }
  setTimeout(() => {
    throw error;
  }, 0);
}

function enqueueActivationChain(target: LazyHydrationTarget): void {
  const scratchStart = ancestorScratch.length;
  let current: Element | null = target;
  while (current) {
    const candidate = current as LazyHydrationTarget;
    if (pendingTargets.has(candidate)) {
      ancestorScratch.push(candidate);
    }
    current = composedParent(current);
  }
  try {
    for (let i = ancestorScratch.length - 1; i >= scratchStart; i--) {
      enqueueActivation(ancestorScratch[i]);
    }
  } finally {
    ancestorScratch.length = scratchStart;
  }
}

function enqueueActivation(target: LazyHydrationTarget): void {
  const generation = activationGenerations.get(target);
  if (
    generation === undefined ||
    queuedGenerations.get(target) === generation
  ) return;
  queuedGenerations.set(target, generation);
  activationQueue.push(target);
  activationQueueGenerations.push(generation);
}

function advanceActivationGeneration(target: LazyHydrationTarget): void {
  activationGenerations.set(
    target,
    (activationGenerations.get(target) ?? 0) + 1,
  );
}

function isCurrentObservation(entry: IntersectionObserverEntry): boolean {
  const started = observationStartTimes.get(
    entry.target as LazyHydrationTarget,
  );
  return started !== undefined &&
    (typeof entry.time !== 'number' || entry.time >= started);
}

function composedParent(element: Element): Element | null {
  if (element.assignedSlot) return element.assignedSlot;
  if (element.parentElement) return element.parentElement;
  const root = element.getRootNode();
  return root instanceof ShadowRoot ? root.host : null;
}

function activate(target: LazyHydrationTarget): void {
  if (!target.isConnected || !pendingTargets.delete(target)) return;
  settleInitialObservation(target);
  if (observedTargets.delete(target)) observer?.unobserve(target);
  removeWakeListenersWhenIdle();

  const streamed = boundaryStates.has(target);
  const state = boundaryStates.get(target);
  boundaryStates.delete(target);
  if (streamed) streamedActivations.add(target);
  try {
    target[LAZY_HYDRATION_ACTIVATE](state);
  } finally {
    if (streamed) streamedActivations.delete(target);
  }
}

function trackInitialObservation(target: LazyHydrationTarget): void {
  if (
    startupObservationGateSealed ||
    !isHydrationStartupPending()
  ) return;
  initialObservationTargets.add(target);
  initialObservationCount++;
  if (startupObservationGateActive) return;
  startupObservationGateActive = true;
  hydrationStart();
  document.addEventListener('DOMContentLoaded', sealStartupObservationGate, {
    once: true,
  });
}

function settleInitialObservation(target: LazyHydrationTarget): void {
  if (!initialObservationTargets.delete(target)) return;
  initialObservationCount--;
  finishStartupObservationGate();
}

function sealStartupObservationGate(): void {
  startupObservationGateSealed = true;
  finishStartupObservationGate();
}

function finishStartupObservationGate(): void {
  if (
    !startupObservationGateActive ||
    !startupObservationGateSealed ||
    initialObservationCount !== 0
  ) return;
  startupObservationGateActive = false;
  hydrationEnd();
}

function handleWakeEvent(event: Event): void {
  const path = event.composedPath();
  // composedPath() is target-first, so reverse iteration activates nested lazy
  // components parent-first before the event reaches its target phase.
  let ownsBatch = false;
  let batchStarted = false;
  try {
    for (let i = path.length - 1; i >= 0; i--) {
      const current = path[i];
      if (current instanceof HTMLElement) {
        const target = current as LazyHydrationTarget;
        if (pendingTargets.has(target)) {
          if (!batchStarted) {
            ownsBatch = beginHydrationBatch();
            batchStarted = true;
          }
          try {
            activate(target);
          } catch (error) {
            reportActivationError(error);
          }
        }
      }
    }
  } finally {
    if (ownsBatch) finishHydrationBatch();
  }
}

function installWakeListeners(): void {
  if (wakeListenersInstalled) return;
  wakeListenersInstalled = true;
  for (let i = 0; i < WAKE_EVENTS.length; i++) {
    document.addEventListener(WAKE_EVENTS[i], handleWakeEvent, true);
  }
}

function removeWakeListenersWhenIdle(): void {
  if (!wakeListenersInstalled || observedTargets.size !== 0) return;
  wakeListenersInstalled = false;
  for (let i = 0; i < WAKE_EVENTS.length; i++) {
    document.removeEventListener(WAKE_EVENTS[i], handleWakeEvent, true);
  }
}
