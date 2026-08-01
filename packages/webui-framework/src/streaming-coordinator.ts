// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Document-scoped FIFO coordinator for progressive hydration checkpoints.
 *
 * Protocol decoding, DOM placement, bootstrap merging, and deferred-root
 * activation live in focused sibling modules. This file owns only stream
 * ordering, failure policy, lifecycle accounting, and terminal validation.
 */

import {
  abortStreamingGate,
  markBoundaryCommitted,
  markBoundaryPending,
} from './lifecycle.js';
import {
  activateRootsBetween,
} from './streaming-activation.js';
import { applyBoundaryBootstrap } from './streaming-bootstrap.js';
import {
  abandonDeferredDocumentRoots,
  abandonDeferredRange,
} from './streaming-cleanup.js';
import {
  abandonPendingWaiters,
  configureStreamingFailureHandler,
  elementHasPendingStateForTests,
  pendingTagWaiterCountForTests,
  pendingUndefinedRootCountForTests,
  resetDeferredActivationForTests,
} from './streaming-deferred.js';
import type { PendingBoundaryUpdates } from './streaming-deferred.js';
import {
  findBoundaryScript,
  findEndMarkerByPrefix,
  findStartMarkerByPrefix,
  removeBoundaryScaffolding,
  resolveBoundaryRange,
  streamingErrorMessage,
} from './streaming-dom.js';
import type { HydrationRange } from './streaming-dom.js';
import {
  parseBoundaryEnvelope,
  RECORD_KIND_STATE_UPDATE,
  RECORD_KIND_TERMINAL,
  RECORD_KIND_UPDATABLE_CHECKPOINT,
} from './streaming-protocol.js';
import type { BoundaryBootstrap } from './streaming-protocol.js';
import { applyStateUpdate } from './streaming-state.js';

const MAX_QUEUED_BOUNDARIES = 512;
const MAX_UPDATABLE_BOUNDARIES = 128;
const MAX_RETAINED_UPDATE_ROOTS = 50_000;
const BOUNDARY_HYDRATED_EVENT = 'webui:boundary-hydrated';
/** `performance.mark()` label prefix. The suffix is the compile-time boundary
 *  ID — its declaration index — so a mark resolves back to the authored
 *  `<boundary name>` through the build manifest without any name string ever
 *  reaching the wire (rule 18). */
const BOUNDARY_MARK_PREFIX = 'webui:boundary:';
const UPDATE_MARK_SUFFIX = ':update';
const TERMINAL_MARK = 'webui:streaming:terminal';

/** Captured once: marks and the slice clock must not re-resolve per commit. */
const perf = (globalThis as { performance?: Performance }).performance;

const queue: Element[] = [];
let queueHead = 0;
let pumpScheduled = false;
/** True only while a time-sliced drain is yielding between boundaries. */
let slicedDrainActive = false;
let halted = false;
let nextExpectedRecordSequence = 0;
let nextExpectedBoundaryId = 0;
let terminalCommitted = false;
let pendingTerminalSequence: number | null = null;
let terminalValidationScheduled = false;
let coordinatorGeneration = 0;
let retainedUpdateRoots = 0;

type UpdatableBoundary = PendingBoundaryUpdates;

const updatableBoundaries = new Map<number, UpdatableBoundary>();

configureStreamingFailureHandler(fail);

/** Enqueue one parser-upgraded sentinel for the shared microtask pump. */
export function enqueueStreamingSentinel(sentinel: Element): void {
  if (halted) {
    discardRejectedBoundary(sentinel);
    abandonDeferredDocumentRoots();
    return;
  }
  if (queue.length - queueHead >= MAX_QUEUED_BOUNDARIES) {
    failBoundary(
      sentinel,
      `queued boundary count exceeds ${MAX_QUEUED_BOUNDARIES}`,
    );
    return;
  }
  queue.push(sentinel);
  if (!pumpScheduled) {
    pumpScheduled = true;
    queueMicrotask(drainQueue);
  }
}

/**
 * Drain the queue.
 *
 * The default is a single uninterrupted pass: it finishes hydration soonest
 * and keeps `webui:hydration-complete` early, which is the right trade for the
 * common case where boundaries arrive spread across the response.
 *
 * Setting `window.__WEBUI_STREAMING_SLICE_MS__` to a positive millisecond
 * budget opts into a time-sliced drain instead. That matters when an
 * intermediary coalesces the response and every boundary lands in one chunk —
 * precisely the single long hydration task streaming exists to avoid. It costs
 * total hydration time and delays the last boundary's interactivity, so it is
 * opt-in rather than the default.
 */
function drainQueue(): void {
  const budget = sliceBudgetMs();
  if (budget > 0) {
    void drainSliced(budget);
    return;
  }
  pumpScheduled = false;
  while (queueHead < queue.length) {
    const sentinel = queue[queueHead];
    queueHead++;
    if (!halted) processSentinel(sentinel);
  }
  queue.length = 0;
  queueHead = 0;
  scheduleTerminalValidation();
}

/** Opt-in millisecond slice budget; `0` keeps the batched drain. */
function sliceBudgetMs(): number {
  const raw = (window as unknown as { __WEBUI_STREAMING_SLICE_MS__?: unknown })
    .__WEBUI_STREAMING_SLICE_MS__;
  return typeof raw === 'number' && raw > 0 ? raw : 0;
}

function nowMs(): number {
  return perf ? perf.now() : Date.now();
}

/**
 * Hand the frame back after each `budget` of commit work.
 *
 * A microtask per boundary would not help — the whole microtask checkpoint
 * drains before the renderer regains control — so this yields with
 * `scheduler.yield()` (or a task) to actually release the main thread.
 *
 * `pumpScheduled` stays true for the whole pass so a sentinel enqueued during
 * a yield joins this drain instead of starting a second, interleaved one. It
 * also keeps `validatePendingTerminal` deferring until the queue is truly
 * drained, so a sliced drain cannot settle the terminal early.
 */
async function drainSliced(budget: number): Promise<void> {
  const generation = coordinatorGeneration;
  slicedDrainActive = true;
  let deadline = nowMs() + budget;
  while (queueHead < queue.length) {
    const sentinel = queue[queueHead];
    queueHead++;
    if (!halted) processSentinel(sentinel);
    if (nowMs() >= deadline) {
      await yieldToRenderer();
      // A reset or a fresh document during the yield abandons this pass.
      if (generation !== coordinatorGeneration) {
        slicedDrainActive = false;
        return;
      }
      deadline = nowMs() + budget;
    }
  }
  slicedDrainActive = false;
  pumpScheduled = false;
  queue.length = 0;
  queueHead = 0;
  scheduleTerminalValidation();
}

function yieldToRenderer(): Promise<void> {
  const scheduler = (globalThis as {
    scheduler?: { yield?: () => Promise<void> };
  }).scheduler;
  if (scheduler?.yield) return scheduler.yield();
  return new Promise<void>((resolve) => {
    setTimeout(resolve, 0);
  });
}

function fail(reason: string): void {
  halted = true;
  console.error(`[WebUI] streaming hydration halted: ${reason}.`);
  // Abort before balancing waiters so malformed streams never dispatch the
  // successful completion event while their failure cleanup drains.
  abortStreamingGate();
  abandonPendingWaiters();
  clearUpdatableBoundaries();
  settlePendingTerminal(false);
  for (let i = queueHead; i < queue.length; i++) {
    discardRejectedBoundary(queue[i]);
  }
  queue.length = 0;
  queueHead = 0;
  abandonDeferredDocumentRoots();
}

function failBoundary(sentinel: Element, reason: string): void {
  discardRejectedBoundary(sentinel);
  fail(reason);
}

function discardRejectedBoundary(sentinel: Element): void {
  const scriptEl = findBoundaryScript(sentinel);
  let endMarker: Comment | null = null;
  let startMarker: Comment | null = null;
  if (scriptEl) {
    endMarker = findEndMarkerByPrefix(scriptEl);
    if (endMarker) startMarker = findStartMarkerByPrefix(endMarker);
  }
  if (startMarker && endMarker) {
    abandonDeferredRange(startMarker, endMarker);
  }
  removeBoundaryScaffolding(
    sentinel,
    scriptEl,
    startMarker,
    endMarker,
  );
}

function processSentinel(sentinel: Element): void {
  if (terminalCommitted) {
    failBoundary(
      sentinel,
      'boundary arrived after the terminal streaming record',
    );
    return;
  }

  const scriptEl = findBoundaryScript(sentinel);
  if (!scriptEl) {
    failBoundary(
      sentinel,
      'missing boundary payload script before <webui-hydrate>',
    );
    return;
  }

  const parsed = parseBoundaryEnvelope(scriptEl.textContent ?? '');
  if (!parsed.ok) {
    failBoundary(sentinel, parsed.reason);
    return;
  }

  const [, sequence, kind, target, payload] = parsed.envelope;
  if (sequence !== nextExpectedRecordSequence) {
    failBoundary(
      sentinel,
      `expected streaming record sequence ${nextExpectedRecordSequence}, received ${sequence}`,
    );
    return;
  }

  if (kind === RECORD_KIND_STATE_UPDATE) {
    const boundary = updatableBoundaries.get(target);
    if (!boundary) {
      failBoundary(
        sentinel,
        `state update targets boundary ${target}, which is not committed as updatable`,
      );
      return;
    }
    nextExpectedRecordSequence++;
    commitStateUpdate(
      boundary,
      payload as Record<string, unknown>,
      target,
      sequence,
      sentinel,
      scriptEl,
    );
    return;
  }

  if (kind === RECORD_KIND_TERMINAL) {
    const resolved = resolveBoundaryRange(scriptEl, 0, true);
    if (!resolved.ok) {
      failBoundary(sentinel, resolved.reason);
      return;
    }
    nextExpectedRecordSequence++;
    commitTerminal(sequence, sentinel, scriptEl);
    return;
  }

  if (target !== nextExpectedBoundaryId) {
    failBoundary(
      sentinel,
      `expected boundary ID ${nextExpectedBoundaryId}, received ${target}`,
    );
    return;
  }
  const resolved = resolveBoundaryRange(scriptEl, target, false);
  if (!resolved.ok) {
    if (resolved.truncated) {
      if (resolved.start) {
        abandonDeferredRange(resolved.start, scriptEl);
      }
      removeBoundaryScaffolding(
        sentinel,
        scriptEl,
        resolved.start,
        null,
      );
      fail(resolved.reason);
    } else {
      failBoundary(sentinel, resolved.reason);
    }
    return;
  }

  nextExpectedRecordSequence++;
  nextExpectedBoundaryId++;
  commitCheckpoint(
    payload as BoundaryBootstrap,
    resolved.range,
    sequence,
    target,
    kind === RECORD_KIND_UPDATABLE_CHECKPOINT,
    sentinel,
    scriptEl,
  );
}

function commitCheckpoint(
  bootstrap: BoundaryBootstrap,
  range: HydrationRange,
  sequence: number,
  target: number,
  updatable: boolean,
  sentinel: Element,
  scriptEl: Element,
): void {
  markBoundaryPending();
  let committed = false;
  try {
    applyBoundaryBootstrap(bootstrap);
    if (range.start && range.end) {
      const boundary: UpdatableBoundary | undefined = updatable
        ? { roots: [], retained: 0, pendingRoots: 0 }
        : undefined;
      activateRootsBetween(
        range.start,
        range.end,
        bootstrap.state,
        boundary,
      );
      if (boundary) retainUpdatableBoundary(target, boundary);
    } else if (updatable) {
      retainUpdatableBoundary(target, {
        roots: [],
        retained: 0,
        pendingRoots: 0,
      });
    }
    committed = true;
  } catch (error) {
    fail(
      `error committing boundary ${sequence}: ${
        streamingErrorMessage(error)
      }`,
    );
  } finally {
    removeBoundaryScaffolding(
      sentinel,
      scriptEl,
      range.start,
      range.end,
    );
    if (committed) {
      notifyCommit(`${BOUNDARY_MARK_PREFIX}${target}`, sequence, 'checkpoint');
    }
    markBoundaryCommitted(false);
  }
}

function retainUpdatableBoundary(
  target: number,
  boundary: UpdatableBoundary,
): void {
  if (updatableBoundaries.size >= MAX_UPDATABLE_BOUNDARIES) {
    throw new Error(
      `updatable boundary count exceeds ${MAX_UPDATABLE_BOUNDARIES}`,
    );
  }
  if (
    retainedUpdateRoots + boundary.retained >
    MAX_RETAINED_UPDATE_ROOTS
  ) {
    throw new Error(
      `retained update root count exceeds ${MAX_RETAINED_UPDATE_ROOTS}`,
    );
  }
  retainedUpdateRoots += boundary.retained;
  updatableBoundaries.set(target, boundary);
}

function commitStateUpdate(
  boundary: UpdatableBoundary,
  patch: Record<string, unknown>,
  target: number,
  sequence: number,
  sentinel: Element,
  scriptEl: Element,
): void {
  try {
    if (boundary.pendingRoots > 0) {
      if (boundary.patch) {
        Object.assign(boundary.patch, patch);
      } else {
        boundary.patch = Object.assign(
          Object.create(null) as Record<string, unknown>,
          patch,
        );
      }
    }
    for (let i = 0; i < boundary.roots.length; i++) {
      const root = boundary.roots[i];
      if (!applyStateUpdate(root, patch)) {
        throw new Error(
          `<${
            root.tagName.toLowerCase()
          }> activated without a setState() method`,
        );
      }
    }
    notifyCommit(
      `${BOUNDARY_MARK_PREFIX}${target}${UPDATE_MARK_SUFFIX}`,
      sequence,
      'update',
    );
  } catch (error) {
    fail(
      `error applying state update to boundary ${target}: ${
        streamingErrorMessage(error)
      }`,
    );
  } finally {
    removeBoundaryScaffolding(sentinel, scriptEl, null, null);
  }
}

function commitTerminal(
  sequence: number,
  sentinel: Element,
  scriptEl: Element,
): void {
  markBoundaryPending();
  removeBoundaryScaffolding(sentinel, scriptEl, null, null);
  terminalCommitted = true;
  pendingTerminalSequence = sequence;
  clearUpdatableBoundaries();
  scheduleTerminalValidation();
}

function clearUpdatableBoundaries(): void {
  updatableBoundaries.clear();
  retainedUpdateRoots = 0;
}

/**
 * Record one committed boundary.
 *
 * The `performance.mark()` is unconditional and carries no listener
 * requirement: a consumer that loads after hydration finished still reads
 * every commit back out of the performance timeline. That is the property an
 * event cannot have — the coordinator installs from a separate async entry, so
 * a listener registered by application code races the first commits and
 * silently misses them.
 *
 * The `CustomEvent` stays debug-gated. It is the live-observation surface;
 * `kind` distinguishes an initial hydration from a later state update, which
 * never rehydrates (rule 15).
 */
function notifyCommit(mark: string, sequence: number, kind: string): void {
  perf?.mark(mark);
  if (
    (window as unknown as { __WEBUI_STREAMING_DEBUG__?: boolean })
      .__WEBUI_STREAMING_DEBUG__ !== true
  ) {
    return;
  }
  window.dispatchEvent(
    new CustomEvent(BOUNDARY_HYDRATED_EVENT, {
      detail: { sequence, terminal: kind === 'terminal', kind },
    }),
  );
}

function scheduleTerminalValidation(): void {
  if (
    pendingTerminalSequence === null ||
    terminalValidationScheduled ||
    // A sliced drain yields on macrotasks, but this validation reschedules
    // itself on a microtask while the queue is non-empty. Arming it mid-drain
    // would spin the microtask checkpoint forever and starve the yield. The
    // drain arms it once, after the queue is drained.
    slicedDrainActive
  ) {
    return;
  }
  terminalValidationScheduled = true;
  const generation = coordinatorGeneration;
  queueMicrotask(() => validatePendingTerminal(generation));
}

function validatePendingTerminal(generation: number): void {
  if (generation !== coordinatorGeneration) return;
  terminalValidationScheduled = false;
  if (pendingTerminalSequence === null) return;
  if (pumpScheduled || queueHead < queue.length) {
    scheduleTerminalValidation();
    return;
  }
  settlePendingTerminal(!halted);
}

function settlePendingTerminal(success: boolean): void {
  const sequence = pendingTerminalSequence;
  if (sequence === null) return;
  pendingTerminalSequence = null;
  if (success) notifyCommit(TERMINAL_MARK, sequence, 'terminal');
  markBoundaryCommitted(success);
}

/** Install the one-shot parser-completion guard for a truncated stream. */
export function installStreamingTruncationGuard(): void {
  if (typeof document === 'undefined') return;
  const generation = coordinatorGeneration;
  if (document.readyState === 'loading') {
    document.addEventListener(
      'DOMContentLoaded',
      () => onDomContentLoaded(generation),
      { once: true },
    );
  } else {
    queueMicrotask(() => onDomContentLoaded(generation));
  }
}

function onDomContentLoaded(generation: number): void {
  if (generation !== coordinatorGeneration) return;
  if (halted) {
    abandonDeferredDocumentRoots();
    return;
  }
  if (terminalCommitted) return;
  fail(
    'response ended at DOMContentLoaded before the terminal streaming boundary committed (truncated stream)',
  );
}

/** Reset coordinator singletons and invalidate queued promise reactions. */
export function resetStreamingCoordinatorStateForTests(): void {
  resetDeferredActivationForTests();
  settlePendingTerminal(false);
  clearUpdatableBoundaries();
  coordinatorGeneration++;
  queue.length = 0;
  queueHead = 0;
  pumpScheduled = false;
  slicedDrainActive = false;
  halted = false;
  nextExpectedRecordSequence = 0;
  nextExpectedBoundaryId = 0;
  terminalCommitted = false;
  pendingTerminalSequence = null;
  terminalValidationScheduled = false;
}

export function isStreamingHaltedForTests(): boolean {
  return halted;
}

export function streamingRetentionStateForTests(): readonly [number, number] {
  return [updatableBoundaries.size, retainedUpdateRoots];
}

export {
  elementHasPendingStateForTests,
  pendingTagWaiterCountForTests,
  pendingUndefinedRootCountForTests,
};
