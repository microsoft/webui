// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  markLateActivationPending,
  settleLateActivation,
} from './lifecycle.js';
import {
  abandonDeferredDescendants,
  abandonDeferredElement,
  removeStreamingAttributes,
} from './streaming-cleanup.js';
import {
  firstNodeWithin,
  MAX_ELEMENTS_PER_BOUNDARY,
  MAX_MARKER_SCAN_NODES,
  nextAfterSubtreeWithin,
  nextWithinRoot,
  STREAMING_ENCLOSING_SPAN_ATTR,
  streamingErrorMessage,
} from './streaming-dom.js';
import {
  ACTIVATION_ACTIVATED,
  ACTIVATION_ANCESTOR_BARRIER,
  ACTIVATION_MISSING_TEMPLATE,
  ACTIVATION_STATIC_HOST_OPT_OUT,
  PENDING_ROOT_CONNECTED,
  STREAMED_HOST_ATTR,
  STREAMING_BOUNDARY_ACTIVATE,
} from './streaming-mode.js';
import { applyStateUpdate } from './streaming-state.js';

// Coordinator-internal walk results, deliberately in a decade disjoint from the
// shared `ACTIVATION_*` outcomes (1..4) declared in `streaming-mode.ts`. Both
// spaces travel through the same `number`, so overlapping them once made an
// ancestor barrier indistinguishable from a definition-deferred element.
export const ELEMENT_IGNORED = 10;
export const ELEMENT_DEFERRED = 11;
export const ELEMENT_LIMIT_FAILURE = 12;
export const ELEMENT_ACTIVATED_FROM_PENDING = 13;
export const ELEMENT_BARRIER_LIMIT_FAILURE = 14;
/** A hook returned something outside the shared activation contract. */
export const ELEMENT_INVALID_OUTCOME = 15;
export const MAX_PENDING_UNDEFINED_ROOTS = 50_000;
export const MAX_PENDING_BARRIER_ROOTS = 50_000;

type BoundaryActivatable = Element & {
  // Typed as `number`, not `ActivationOutcome`: the hook may belong to a
  // foreign element that never saw this contract, so the value is validated
  // once in `invokeActivationHook` instead of being trusted by the type.
  [STREAMING_BOUNDARY_ACTIVATE]?: (
    state?: Record<string, unknown>,
    bypassAncestor?: Element,
  ) => number;
};

/**
 * The one unfinished component host an early boundary's marked roots may skip.
 *
 * Allocated once per boundary that declares an enclosing span, never per root.
 * `id` is the canonical decimal SpanInstanceId the compiler wrote onto both the
 * host (`data-ws-span`) and the entitled early roots (`data-ws-enclosing`);
 * `host` is the element that attribute already resolved to. Matching here and
 * handing the element itself to the activation hook is what keeps every span
 * attribute name out of the always-shipped bundle.
 */
export interface SpanBypass {
  readonly id: string;
  readonly host: Element;
}

// Both host kinds opt out for marker cleanup, but only the full dormant host
// exposes setState and can accept later boundary patches.
function retainsBoundaryUpdates(el: Element, outcome: number): boolean {
  return outcome === ACTIVATION_ACTIVATED
    || (
      outcome === ACTIVATION_STATIC_HOST_OPT_OUT
      && typeof (el as Element & { setState?: unknown }).setState === 'function'
    );
}

interface PendingTagWaiter {
  readonly generation: number;
  readonly roots: Set<Element>;
}

/**
 * Everything one deferred root must replay when it finally activates.
 *
 * Held as a single symbol-keyed record rather than three parallel properties:
 * one hidden-class transition per retained root instead of three, one delete
 * instead of three, and no absent-vs-`undefined` sentinels.
 */
interface PendingRootRecord {
  readonly state: Record<string, unknown> | undefined;
  readonly updates: PendingBoundaryUpdates | undefined;
  readonly bypass: SpanBypass | undefined;
}

const pendingTagWaiters = new Map<string, PendingTagWaiter>();
const pendingBarrierRoots = new Set<Element>();
let pendingUndefinedRoots = 0;
let activationGeneration = 0;
let failureHandler: ((reason: string) => void) | null = null;
/**
 * The offending value behind the most recent `ELEMENT_INVALID_OUTCOME`.
 *
 * A scalar rather than a carried payload so the rejection costs no allocation
 * on a path the coordinator takes for every marked root. Every caller reads it
 * in the same turn it observes the result, before any further hook can run.
 */
let invalidActivationOutcome: unknown;

const PENDING_RECORD = Symbol();

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
   * True while the response can still address this target. A successful
   * terminal clears it but may leave `patch` alive until already-pending roots
   * replay that last committed state. Fatal cleanup clears both.
   */
  active: boolean;
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
  /** Span barrier this boundary's compiler-marked early roots may bypass. */
  bypass?: SpanBypass;
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

function tagOf(el: Element): string {
  return el.tagName.toLowerCase();
}

function missingTemplateReason(tag: string): string {
  return `template metadata missing while activating <${tag}>`;
}

/**
 * Report a hook that answered outside the shared activation contract.
 *
 * Kept distinct from `missingTemplateReason` on purpose: folding an
 * unrecognized code into "missing template" sends every reader looking for
 * absent metadata when the real defect is a hook returning a code this
 * coordinator cannot decode.
 */
function invalidOutcomeReason(tag: string): string {
  return `<${tag}> returned an unrecognized streaming activation outcome ${
    String(invalidActivationOutcome)
  }`;
}

function barrierLimitReason(): string {
  return `pending ancestor-barrier root count exceeds ${MAX_PENDING_BARRIER_ROOTS}`;
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
  fail(`<${tagOf(el)}> activated without a setState() method`);
}

/** Join one activated root to its boundary and replay any collapsed patch. */
function joinBoundaryUpdates(
  el: Element,
  updates: PendingBoundaryUpdates,
): void {
  // Joining only on a known-good outcome is what keeps a failed or ignored
  // element out of the update set for the life of the page.
  if (updates.active) updates.roots.push(el);
  // Replayed rather than merged into hydration state: `$hydrate` wires
  // bindings against the server's bytes without evaluating them, so seeding a
  // post-render value first would bind the old branch while the element
  // believed it held the new one.
  if (updates.patch) requireStateUpdate(el, updates.patch);
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
  bypass?: SpanBypass,
): number {
  const tag = tagOf(el);
  if (tag.indexOf('-') === -1) return ELEMENT_IGNORED;

  if (customElements.get(tag)) {
    if (pendingBarrierRoots.has(el)) {
      return activatePendingBarrierRoot(el);
    }
    // A definition waiter still owns this root until its shared reaction runs.
    // Consuming its state here would leave the waiter count and lifecycle stuck.
    if (hasPendingRecord(el)) return ELEMENT_DEFERRED;
    const outcome = invokeActivationHook(el, state, bypass);
    if (outcome !== ACTIVATION_ANCESTOR_BARRIER) return outcome;
    if (pendingBarrierRoots.size >= MAX_PENDING_BARRIER_ROOTS) {
      return ELEMENT_BARRIER_LIMIT_FAILURE;
    }
    deferBehindBarrier(el, state, updates, bypass);
    return ELEMENT_DEFERRED;
  }

  if (!hasPendingRecord(el)) {
    if (pendingUndefinedRoots >= MAX_PENDING_UNDEFINED_ROOTS) {
      return ELEMENT_LIMIT_FAILURE;
    }
    stashPendingRecord(el, state, updates, bypass);
  }
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

/** Retain one root whose hook reported an unfinished ancestor barrier. */
function deferBehindBarrier(
  el: Element,
  state: Record<string, unknown> | undefined,
  updates: PendingBoundaryUpdates | undefined,
  bypass: SpanBypass | undefined,
): void {
  stashPendingRecord(el, state, updates, bypass);
  pendingBarrierRoots.add(el);
  (el as PendingRoot)[PENDING_ROOT_CONNECTED] = resumeBarrierRoot;
}

function activatePendingBarrierRoot(el: Element): number {
  if (!pendingBarrierRoots.delete(el)) return ELEMENT_IGNORED;
  delete (el as PendingRoot)[PENDING_ROOT_CONNECTED];
  const record = takePendingRecord(el);
  const updates = releaseUpdates(record);
  try {
    const outcome = resumeRetainedRoot(el, record, updates);
    if (outcome === ACTIVATION_ANCESTOR_BARRIER) return ELEMENT_DEFERRED;
    return outcome === ACTIVATION_MISSING_TEMPLATE ||
        outcome === ELEMENT_INVALID_OUTCOME
      ? outcome
      : ELEMENT_ACTIVATED_FROM_PENDING;
  } finally {
    if (updates?.pendingRoots === 0) updates.patch = undefined;
  }
}

/**
 * Re-run one retained root's activation and hand back the raw outcome.
 *
 * Shared by both retention reasons — an undefined tag and an unfinished
 * ancestor barrier — because they differ only in the bookkeeping around the
 * call. A root still behind a barrier is re-retained here so neither caller
 * can forget to, and an activated root joins its boundary parent-first,
 * before any descendant walk.
 */
function resumeRetainedRoot(
  el: Element,
  record: PendingRootRecord | undefined,
  updates: PendingBoundaryUpdates | undefined,
): number {
  const outcome = invokeActivationHook(el, record?.state, record?.bypass);
  if (outcome === ACTIVATION_ANCESTOR_BARRIER) {
    deferBehindBarrier(el, record?.state, updates, record?.bypass);
  } else if (updates && retainsBoundaryUpdates(el, outcome)) {
    joinBoundaryUpdates(el, updates);
  }
  return outcome;
}

/** Resume coordinator-owned activation when a component barrier releases. */
function resumeBarrierRoot(this: Element): void {
  try {
    const outcome = activatePendingBarrierRoot(this);
    if (outcome === ACTIVATION_MISSING_TEMPLATE) {
      abandonDeferredTree(this);
      fail(missingTemplateReason(tagOf(this)));
    } else if (outcome === ELEMENT_INVALID_OUTCOME) {
      abandonDeferredTree(this);
      fail(invalidOutcomeReason(tagOf(this)));
    }
  } catch (error) {
    abandonDeferredDescendants(this);
    reportActivationFailure(tagOf(this), error);
  }
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
  const tag = tagOf(this);
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
  const record = takePendingRecord(el);
  const updates = releaseUpdates(record);
  const state = record?.state;
  const bypass = record?.bypass;
  try {
    const outcome = resumeRetainedRoot(el, record, updates);
    if (outcome === ACTIVATION_MISSING_TEMPLATE) {
      abandonDeferredTree(el);
      fail(missingTemplateReason(tag));
      return;
    }
    if (outcome === ELEMENT_INVALID_OUTCOME) {
      abandonDeferredTree(el);
      fail(invalidOutcomeReason(tag));
      return;
    }
    if (outcome === ACTIVATION_ANCESTOR_BARRIER) {
      // Re-retained by `resumeRetainedRoot`; only the budget is enforced here.
      if (pendingBarrierRoots.size > MAX_PENDING_BARRIER_ROOTS) {
        abandonDeferredTree(el);
        fail(barrierLimitReason());
      }
      return;
    }
    const failure = activateDeferredTree(
      firstNodeWithin(el),
      el,
      null,
      state,
      updates || bypass ? { updates, bypass } : undefined,
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
  const bypass = options?.bypass;
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
          const outcome = activateMarkedElement(el, state, updates, bypass);
          if (outcome === ACTIVATION_MISSING_TEMPLATE) {
            return missingTemplateReason(tagOf(el));
          }
          if (outcome === ELEMENT_INVALID_OUTCOME) {
            return invalidOutcomeReason(tagOf(el));
          }
          if (outcome === ELEMENT_LIMIT_FAILURE) {
            return `pending undefined root count exceeds ${MAX_PENDING_UNDEFINED_ROOTS}`;
          }
          if (outcome === ELEMENT_BARRIER_LIMIT_FAILURE) {
            return barrierLimitReason();
          }
          if (outcome === ELEMENT_DEFERRED) {
            resumeAfterDeferred = nextAfterSubtreeWithin(node, root);
            skippingDeferredDescendants = true;
          } else if (
            updates &&
            retainsBoundaryUpdates(el, outcome)
          ) {
            joinBoundaryUpdates(el, updates);
          }
        } catch (error) {
          reportActivationFailure(tagOf(el), error);
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

/** Release one failed root together with the subtree it was gating. */
function abandonDeferredTree(el: Element): void {
  abandonDeferredDescendants(el);
  abandonDeferredElement(el);
}

/** Balance and clear every pending undefined-tag waiter exactly once. */
export function abandonPendingWaiters(): void {
  if (pendingBarrierRoots.size !== 0) {
    for (const el of pendingBarrierRoots) clearPendingRoot(el);
    pendingBarrierRoots.clear();
  }
  if (pendingTagWaiters.size !== 0) {
    for (const waiter of pendingTagWaiters.values()) {
      for (const el of waiter.roots) clearPendingRoot(el);
      settleLateActivation();
    }
    pendingTagWaiters.clear();
  }
  pendingUndefinedRoots = 0;
}

function clearPendingRoot(el: Element): void {
  releaseUpdates(takePendingRecord(el));
  delete (el as PendingRoot)[PENDING_ROOT_CONNECTED];
  abandonDeferredTree(el);
}

function stashPendingRecord(
  el: Element,
  state: Record<string, unknown> | undefined,
  updates: PendingBoundaryUpdates | undefined,
  bypass: SpanBypass | undefined,
): void {
  (el as unknown as Record<symbol, PendingRootRecord>)[PENDING_RECORD] = {
    state,
    updates,
    bypass,
  };
  if (updates) updates.pendingRoots++;
}

function hasPendingRecord(el: Element): boolean {
  return Object.prototype.hasOwnProperty.call(el, PENDING_RECORD);
}

function takePendingRecord(el: Element): PendingRootRecord | undefined {
  const store = el as unknown as Record<symbol, PendingRootRecord | undefined>;
  const record = store[PENDING_RECORD];
  delete store[PENDING_RECORD];
  return record;
}

/**
 * Balance one taken record against its boundary's pending-root accounting.
 *
 * Returns the boundary only while it can still receive this root: a terminal
 * that already dropped both the live set and the collapsed patch leaves
 * nothing to join or replay.
 */
function releaseUpdates(
  record: PendingRootRecord | undefined,
): PendingBoundaryUpdates | undefined {
  const updates = record?.updates;
  if (!updates) return undefined;
  updates.pendingRoots--;
  return updates.active || updates.patch !== undefined ? updates : undefined;
}

function invokeActivationHook(
  el: Element,
  state: Record<string, unknown> | undefined,
  bypass: SpanBypass | undefined,
): number {
  const hook = (el as BoundaryActivatable)[STREAMING_BOUNDARY_ACTIVATE];
  if (typeof hook !== 'function') return ACTIVATION_MISSING_TEMPLATE;
  // The compiler entitles a specific set of early roots to skip the enclosing
  // span host, so the attribute is matched here and the resolved element is
  // handed over. One read, and only for a boundary that declares a span.
  const bypassAncestor = bypass !== undefined &&
      el.getAttribute(STREAMING_ENCLOSING_SPAN_ATTR) === bypass.id
    ? bypass.host
    : undefined;
  let outcome: number;
  try {
    outcome = hook.call(el, state, bypassAncestor);
  } catch (error) {
    removeStreamingAttributes(el);
    throw error;
  }
  // Only a genuinely finished root gives up its markers. A barrier still owns
  // its root, and a root that could not hydrate keeps them for fatal cleanup.
  if (
    outcome === ACTIVATION_ACTIVATED ||
    outcome === ACTIVATION_STATIC_HOST_OPT_OUT
  ) {
    removeStreamingAttributes(el);
    return outcome;
  }
  if (
    outcome === ACTIVATION_ANCESTOR_BARRIER ||
    outcome === ACTIVATION_MISSING_TEMPLATE
  ) {
    return outcome;
  }
  invalidActivationOutcome = outcome;
  return ELEMENT_INVALID_OUTCOME;
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

export function pendingBarrierRootCountForTests(): number {
  return pendingBarrierRoots.size;
}

export function elementHasPendingStateForTests(el: Element): boolean {
  return hasPendingRecord(el);
}
