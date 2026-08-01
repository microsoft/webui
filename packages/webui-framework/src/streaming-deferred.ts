// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  markLateActivationPending,
  settleLateActivation,
} from './lifecycle.js';
import {
  abandonDeferredDescendants,
  abandonDeferredElement,
} from './streaming-cleanup.js';
import {
  firstNodeWithin,
  MAX_ELEMENTS_PER_BOUNDARY,
  MAX_MARKER_SCAN_NODES,
  nextAfterSubtreeWithin,
  nextWithinRoot,
  safeRemoveAttribute,
  streamingErrorMessage,
} from './streaming-dom.js';
import {
  PENDING_ROOT_CONNECTED,
  STREAMED_HOST_ATTR,
  STREAMING_BOUNDARY_ACTIVATE,
} from './streaming-mode.js';
import { applyStateUpdate } from './streaming-state.js';

const ACTIVATION_ACTIVATED = 1;
const ACTIVATION_STATIC_HOST_OPT_OUT = 2;
export const ACTIVATION_MISSING_TEMPLATE = 3;
export const ELEMENT_IGNORED = 0;
export const ELEMENT_DEFERRED = 4;
export const ELEMENT_LIMIT_FAILURE = 5;
export const MAX_PENDING_UNDEFINED_ROOTS = 50_000;

type BoundaryActivatable = Element & {
  [STREAMING_BOUNDARY_ACTIVATE]?: (
    state?: Record<string, unknown>,
  ) => number;
};

interface PendingTagWaiter {
  readonly generation: number;
  readonly roots: Set<Element>;
}

const pendingTagWaiters = new Map<string, PendingTagWaiter>();
let pendingUndefinedRoots = 0;
let activationGeneration = 0;
let failureHandler: ((reason: string) => void) | null = null;

const PENDING_BOUNDARY_STATE = Symbol();
const PENDING_BOUNDARY_UPDATES = Symbol();
const NO_BOUNDARY_STATE: unique symbol = Symbol();

/** One boundary-owned shallow patch shared by every deferred root. */
export interface PendingBoundaryUpdates {
  /**
   * Live roots only — a root joins on successful activation and never joins
   * otherwise. `data-ws` cannot answer this: it is a work marker the
   * coordinator strips on success, on hook throw, and on abandon alike, so its
   * absence means "finished with", not "activated".
   */
  readonly roots: Element[];
  /**
   * Marked roots seen by the checkpoint scan, including ones still deferred or
   * destined to fail. Bounds retention pessimistically so a boundary cannot
   * grow past its budget by activating roots after it was retained.
   */
  retained: number;
  pendingRoots: number;
  patch?: Record<string, unknown>;
}

export interface DeferredActivationOptions {
  updates?: PendingBoundaryUpdates;
  /**
   * Set only by the checkpoint scan, which owns the boundary's retention
   * budget. A late activation re-walks a subtree the scan already counted, so
   * counting there again would charge the same roots twice.
   */
  countRetention?: boolean;
}

type PendingRoot = Element & {
  [PENDING_ROOT_CONNECTED]?: () => void;
};

/** Install the coordinator's fatal-error callback once for this document. */
export function configureStreamingFailureHandler(
  handler: (reason: string) => void,
): void {
  failureHandler = handler;
}

function fail(reason: string): void {
  if (!failureHandler) {
    throw new Error('streaming failure handler is not installed');
  }
  failureHandler(reason);
}

/**
 * Deliver a replayed patch, halting when the target cannot accept one.
 *
 * Only successfully activated roots reach this, and `setState` is defined on
 * `TemplateElement` itself, so a missing one means something registered a
 * boundary activation hook without the reactive surface behind it. The
 * coordinator has already promised this boundary an update it can no longer
 * deliver, and a page that silently drops every future update is worse than a
 * loud stop.
 */
function requireStateUpdate(
  el: Element,
  patch: Record<string, unknown>,
): void {
  if (applyStateUpdate(el, patch)) return;
  fail(
    `<${el.tagName.toLowerCase()}> activated without a setState() method`,
  );
}

/**
 * Activate one marked root or retain it behind one per-tag definition waiter.
 *
 * The caller has already confirmed the `STREAMED_HOST_ATTR` marker. Re-reading
 * it here would double the attribute lookups on the boundary scan, which is the
 * hottest loop in streaming hydration.
 */
function activateMarkedElement(
  el: Element,
  state: Record<string, unknown> | undefined,
  updates?: PendingBoundaryUpdates,
): number {
  const tag = el.tagName.toLowerCase();
  if (tag.indexOf('-') === -1) return ELEMENT_IGNORED;

  if (customElements.get(tag)) return invokeActivationHook(el, state);
  if (
    !hasPendingState(el) &&
    pendingUndefinedRoots >= MAX_PENDING_UNDEFINED_ROOTS
  ) {
    return ELEMENT_LIMIT_FAILURE;
  }

  stashPendingState(el, state, updates);
  let waiter = pendingTagWaiters.get(tag);
  if (!waiter) {
    waiter = { generation: activationGeneration, roots: new Set() };
    pendingTagWaiters.set(tag, waiter);
    markLateActivationPending();
    const generation = activationGeneration;
    customElements
      .whenDefined(tag)
      .then(() => onTagDefined(tag, generation));
  }
  if (!waiter.roots.has(el)) {
    waiter.roots.add(el);
    pendingUndefinedRoots++;
  }
  (el as PendingRoot)[PENDING_ROOT_CONNECTED] = resumePendingRoot;
  return ELEMENT_DEFERRED;
}

function onTagDefined(tag: string, generation: number): void {
  if (generation !== activationGeneration) return;
  const waiter = pendingTagWaiters.get(tag);
  if (!waiter || waiter.generation !== generation) return;

  for (const el of waiter.roots) {
    if (!el.isConnected) {
      try {
        customElements.upgrade(el);
      } catch (error) {
        fail(
          `failed to upgrade detached <${tag}>: ${
            streamingErrorMessage(error)
          }`,
        );
        return;
      }
      const hook = (el as BoundaryActivatable)[STREAMING_BOUNDARY_ACTIVATE];
      if (
        el.hasAttribute(STREAMED_HOST_ATTR) &&
        typeof hook !== 'function'
      ) {
        fail(
          `upgrading detached <${tag}> did not install its streaming activation hook`,
        );
        return;
      }
    }
    activatePendingRoot(tag, waiter, el);
  }
}

/** Shared reconnect seam for roots that were undefined at checkpoint time. */
function resumePendingRoot(this: Element): void {
  const tag = this.tagName.toLowerCase();
  const waiter = pendingTagWaiters.get(tag);
  if (
    !waiter ||
    waiter.generation !== activationGeneration ||
    !waiter.roots.has(this)
  ) {
    delete (this as PendingRoot)[PENDING_ROOT_CONNECTED];
    return;
  }
  activatePendingRoot(tag, waiter, this);
}

function activatePendingRoot(
  tag: string,
  waiter: PendingTagWaiter,
  el: Element,
): void {
  if (!waiter.roots.delete(el)) return;
  pendingUndefinedRoots--;
  delete (el as PendingRoot)[PENDING_ROOT_CONNECTED];
  let updates: PendingBoundaryUpdates | undefined;
  try {
    updates = takePendingUpdates(el);
    const state = takePendingState(el);
    const outcome = invokeActivationHook(el, state);
    if (outcome === ACTIVATION_MISSING_TEMPLATE) {
      abandonDeferredDescendants(el);
      abandonDeferredElement(el);
      fail(`template metadata missing while activating <${tag}>`);
      return;
    }
    // Parent first, and before the descendant walk: this root's own patch may
    // tear down the branch its retained descendants live in, and activating a
    // root inside an already-discarded branch is worse than never reaching it.
    if (updates) {
      updates.roots.push(el);
      if (updates.patch) requireStateUpdate(el, updates.patch);
    }
    const failure = activateDeferredTree(
      firstNodeWithin(el),
      el,
      null,
      state,
      updates ? { updates } : undefined,
    );
    if (failure) fail(failure);
  } catch (error) {
    abandonDeferredDescendants(el);
    reportActivationFailure(tag, error);
  } finally {
    if (updates?.pendingRoots === 0) updates.patch = undefined;
    if (
      waiter.roots.size === 0 &&
      pendingTagWaiters.get(tag) === waiter
    ) {
      pendingTagWaiters.delete(tag);
      settleLateActivation();
    }
  }
}

/**
 * Activate one bounded DOM range while preserving undefined-parent barriers.
 *
 * `root` bounds traversal. `end` is the exclusive marker for a boundary walk
 * and `null` when walking a retained element subtree.
 */
export function activateDeferredTree(
  first: Node | null,
  root: Node,
  end: Node | null,
  state: Record<string, unknown> | undefined,
  options?: DeferredActivationOptions,
): string | null {
  // Hoisted out of the walk: these are read once per node otherwise, and this
  // loop runs over every node of every boundary.
  const updates = options?.updates;
  const countRetention =
    options?.countRetention === true && updates !== undefined;
  let node = first;
  let resumeAfterDeferred: Node | null = null;
  let skippingDeferredDescendants = false;
  let visited = 0;
  let elements = 0;
  while (node && node !== end) {
    if (visited >= MAX_MARKER_SCAN_NODES) {
      return `streaming boundary walk exceeds ${MAX_MARKER_SCAN_NODES} nodes`;
    }
    visited++;
    const isElement = node.nodeType === 1 /* ELEMENT_NODE */;
    // One attribute read per element, shared by the retention count below and
    // the activation call further down.
    const marked = isElement &&
      (node as Element).hasAttribute(STREAMED_HOST_ATTR);
    if (countRetention && marked) {
      if (updates.retained >= MAX_ELEMENTS_PER_BOUNDARY) {
        return `updatable streaming boundary exceeds ${MAX_ELEMENTS_PER_BOUNDARY} roots`;
      }
      updates.retained++;
    }
    if (
      skippingDeferredDescendants &&
      node !== resumeAfterDeferred
    ) {
      node = nextWithinRoot(node, root);
      continue;
    }
    skippingDeferredDescendants = false;
    if (isElement) {
      if (elements >= MAX_ELEMENTS_PER_BOUNDARY) {
        return `streaming boundary exceeds ${MAX_ELEMENTS_PER_BOUNDARY} elements`;
      }
      elements++;
      // An unmarked element is never a streaming root, so it never needs the
      // call at all -- and in a typical boundary most elements are unmarked.
      if (marked) {
        const el = node as Element;
        try {
          const outcome = activateMarkedElement(el, state, updates);
          if (outcome === ACTIVATION_MISSING_TEMPLATE) {
            return `template metadata missing while activating <${
              el.tagName.toLowerCase()
            }>`;
          }
          if (outcome === ELEMENT_LIMIT_FAILURE) {
            return `pending undefined root count exceeds ${MAX_PENDING_UNDEFINED_ROOTS}`;
          }
          if (outcome === ELEMENT_DEFERRED) {
            resumeAfterDeferred = nextAfterSubtreeWithin(node, root);
            skippingDeferredDescendants = true;
          } else if (
            updates &&
            (outcome === ACTIVATION_ACTIVATED ||
              outcome === ACTIVATION_STATIC_HOST_OPT_OUT)
          ) {
            // Joining here, on a known-good outcome, is what keeps a failed or
            // ignored element out of the update set for the life of the page.
            updates.roots.push(el);
            // Replayed rather than merged into hydration state: `$hydrate` wires
            // bindings against the server's bytes without evaluating them, so
            // seeding a post-render value first would bind the old branch while
            // the element believed it held the new one.
            if (updates.patch) {
              requireStateUpdate(el, updates.patch);
            }
          }
        } catch (error) {
          reportActivationFailure(el.tagName.toLowerCase(), error);
        }
      }
    }
    node = nextWithinRoot(node, root);
  }
  return end && node !== end
    ? 'streaming boundary end marker became unreachable during activation'
    : null;
}

function reportActivationFailure(tag: string, error: unknown): void {
  console.error(
    `[WebUI] streaming: late activation failed for <${tag}>: ${
      streamingErrorMessage(error)
    }`,
  );
}

/** Balance and clear every pending undefined-tag waiter exactly once. */
export function abandonPendingWaiters(): void {
  if (pendingTagWaiters.size === 0) {
    pendingUndefinedRoots = 0;
    return;
  }
  for (const waiter of pendingTagWaiters.values()) {
    for (const el of waiter.roots) clearPendingRoot(el);
    settleLateActivation();
  }
  pendingTagWaiters.clear();
  pendingUndefinedRoots = 0;
}

function clearPendingRoot(el: Element): void {
  if (hasPendingState(el)) takePendingState(el);
  takePendingUpdates(el);
  delete (el as PendingRoot)[PENDING_ROOT_CONNECTED];
  abandonDeferredDescendants(el);
  abandonDeferredElement(el);
}

function stashPendingState(
  el: Element,
  state: Record<string, unknown> | undefined,
  updates?: PendingBoundaryUpdates,
): void {
  const store = el as unknown as Record<symbol, unknown>;
  store[PENDING_BOUNDARY_STATE] =
    state === undefined ? NO_BOUNDARY_STATE : state;
  if (updates) {
    store[PENDING_BOUNDARY_UPDATES] = updates;
    updates.pendingRoots++;
  }
}

function hasPendingState(el: Element): boolean {
  return Object.prototype.hasOwnProperty.call(
    el,
    PENDING_BOUNDARY_STATE,
  );
}

function takePendingState(
  el: Element,
): Record<string, unknown> | undefined {
  const store = el as unknown as Record<symbol, unknown>;
  const stored = store[PENDING_BOUNDARY_STATE];
  delete store[PENDING_BOUNDARY_STATE];
  return stored === NO_BOUNDARY_STATE
    ? undefined
    : (stored as Record<string, unknown> | undefined);
}

function takePendingUpdates(el: Element): PendingBoundaryUpdates | undefined {
  const store = el as unknown as Record<symbol, unknown>;
  const updates = store[PENDING_BOUNDARY_UPDATES] as
    | PendingBoundaryUpdates
    | undefined;
  delete store[PENDING_BOUNDARY_UPDATES];
  if (updates) updates.pendingRoots--;
  return updates;
}

function invokeActivationHook(
  el: Element,
  state: Record<string, unknown> | undefined,
): number {
  const hook = (el as BoundaryActivatable)[STREAMING_BOUNDARY_ACTIVATE];
  if (typeof hook !== 'function') return ACTIVATION_MISSING_TEMPLATE;
  let outcome: number;
  try {
    outcome = hook.call(el, state);
  } catch (error) {
    safeRemoveAttribute(el, STREAMED_HOST_ATTR);
    throw error;
  }
  if (
    outcome !== ACTIVATION_ACTIVATED &&
    outcome !== ACTIVATION_STATIC_HOST_OPT_OUT
  ) {
    return ACTIVATION_MISSING_TEMPLATE;
  }
  safeRemoveAttribute(el, STREAMED_HOST_ATTR);
  return outcome;
}

/** Reset retained activation state and invalidate uncancellable waiters. */
export function resetDeferredActivationForTests(): void {
  abandonPendingWaiters();
  activationGeneration++;
}

export function pendingTagWaiterCountForTests(): number {
  return pendingTagWaiters.size;
}

export function pendingUndefinedRootCountForTests(): number {
  return pendingUndefinedRoots;
}

export function elementHasPendingStateForTests(el: Element): boolean {
  return hasPendingState(el);
}
