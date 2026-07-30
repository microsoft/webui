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
import {
  findBoundaryScript,
  findEndMarkerByPrefix,
  findStartMarkerByPrefix,
  removeBoundaryScaffolding,
  resolveBoundaryRange,
} from './streaming-dom.js';
import type { HydrationRange } from './streaming-dom.js';
import { parseBoundaryEnvelope } from './streaming-protocol.js';
import type { BoundaryBootstrap } from './streaming-protocol.js';

const MAX_QUEUED_BOUNDARIES = 512;
const BOUNDARY_HYDRATED_EVENT = 'webui:boundary-hydrated';

const queue: Element[] = [];
let queueHead = 0;
let pumpScheduled = false;
let halted = false;
let nextExpectedSequence = 0;
let terminalCommitted = false;
let pendingTerminalSequence: number | null = null;
let terminalValidationScheduled = false;
let coordinatorGeneration = 0;

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

  const [, sequence, terminal, bootstrap] = parsed.envelope;
  if (sequence !== nextExpectedSequence) {
    failBoundary(
      sentinel,
      `expected boundary sequence ${nextExpectedSequence}, received ${sequence}`,
    );
    return;
  }

  const resolved = resolveBoundaryRange(
    scriptEl,
    sequence,
    terminal === 1,
  );
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

  nextExpectedSequence++;
  commitBoundary(
    bootstrap,
    resolved.range,
    sequence,
    terminal === 1,
    sentinel,
    scriptEl,
  );
}

function commitBoundary(
  bootstrap: BoundaryBootstrap,
  range: HydrationRange,
  sequence: number,
  terminal: boolean,
  sentinel: Element,
  scriptEl: Element,
): void {
  markBoundaryPending();
  let committed = false;
  try {
    applyBoundaryBootstrap(bootstrap);
    if (range.start && range.end) {
      activateRootsBetween(range.start, range.end, bootstrap.state);
    }
    committed = true;
  } catch (error) {
    fail(
      `error committing boundary ${sequence}: ${
        error instanceof Error ? error.message : String(error)
      }`,
    );
  } finally {
    removeBoundaryScaffolding(
      sentinel,
      scriptEl,
      range.start,
      range.end,
    );
    if (terminal && committed) {
      terminalCommitted = true;
      pendingTerminalSequence = sequence;
      scheduleTerminalValidation();
    } else {
      if (committed) dispatchBoundaryHydrated(sequence, false);
      markBoundaryCommitted(false);
    }
  }
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
  coordinatorGeneration++;
  queue.length = 0;
  queueHead = 0;
  pumpScheduled = false;
  halted = false;
  nextExpectedSequence = 0;
  terminalCommitted = false;
  pendingTerminalSequence = null;
  terminalValidationScheduled = false;
}

export function isStreamingHaltedForTests(): boolean {
  return halted;
}

export {
  elementHasPendingStateForTests,
  pendingTagWaiterCountForTests,
  pendingUndefinedRootCountForTests,
};
