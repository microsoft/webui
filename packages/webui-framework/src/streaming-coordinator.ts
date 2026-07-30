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

const MAX_QUEUED_BOUNDARIES = 512;
const MAX_UPDATABLE_BOUNDARIES = 128;
const MAX_RETAINED_UPDATE_ROOTS = 50_000;
const BOUNDARY_HYDRATED_EVENT = 'webui:boundary-hydrated';

const queue: Element[] = [];
let queueHead = 0;
let pumpScheduled = false;
let halted = false;
let nextExpectedRecordSequence = 0;
let nextExpectedBoundaryId = 0;
let terminalCommitted = false;
let pendingTerminalSequence: number | null = null;
let terminalValidationScheduled = false;
let coordinatorGeneration = 0;
let retainedUpdateRoots = 0;

type UpdatableBoundary = PendingBoundaryUpdates;

type StateRoot = Element & {
  setState?: (state: Record<string, unknown>) => void;
};

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

function drainQueue(): void {
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
        ? { roots: [], pendingRoots: 0 }
        : undefined;
      activateRootsBetween(
        range.start,
        range.end,
        bootstrap.state,
        boundary,
      );
      if (boundary) retainUpdatableBoundary(target, boundary);
    } else if (updatable) {
      retainUpdatableBoundary(target, { roots: [], pendingRoots: 0 });
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
    if (committed) dispatchBoundaryHydrated(sequence, false);
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
    retainedUpdateRoots + boundary.roots.length >
    MAX_RETAINED_UPDATE_ROOTS
  ) {
    throw new Error(
      `retained update root count exceeds ${MAX_RETAINED_UPDATE_ROOTS}`,
    );
  }
  retainedUpdateRoots += boundary.roots.length;
  updatableBoundaries.set(target, boundary);
}

function commitStateUpdate(
  boundary: UpdatableBoundary,
  patch: Record<string, unknown>,
  target: number,
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
      const root = boundary.roots[i] as StateRoot;
      if (root.hasAttribute('data-ws')) continue;
      if (typeof root.setState !== 'function') {
        throw new Error(
          `activated <${root.tagName.toLowerCase()}> has no setState() method`,
        );
      }
      root.setState(patch);
    }
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

function dispatchBoundaryHydrated(
  sequence: number,
  terminal: boolean,
): void {
  if (
    (window as unknown as { __WEBUI_STREAMING_DEBUG__?: boolean })
      .__WEBUI_STREAMING_DEBUG__ !== true
  ) {
    return;
  }
  window.dispatchEvent(
    new CustomEvent(BOUNDARY_HYDRATED_EVENT, {
      detail: { sequence, terminal },
    }),
  );
}

function scheduleTerminalValidation(): void {
  if (
    pendingTerminalSequence === null ||
    terminalValidationScheduled
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
  if (success) dispatchBoundaryHydrated(sequence, true);
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
