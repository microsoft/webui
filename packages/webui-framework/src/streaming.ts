// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Progressive streaming hydration coordinator.
 *
 * The server streams the document as a sequence of *boundaries*. A boundary
 * that introduces new SSR roots wraps them in comment markers; every
 * boundary (whether or not it introduces roots) ends with a JSON envelope
 * and a `<webui-hydrate>` sentinel element:
 *
 *   <!--wb:N--> ...complete SSR roots... <!--/wb:N-->
 *   <script type="application/json" data-webui-boundary>[version,seq,terminal,bootstrap]</script>
 *   <webui-hydrate></webui-hydrate>
 *
 * `bootstrap` has the same shape as `window.__webui` (optional `state`,
 * `templates`, `inventory`, ...). Sequences are monotonic from zero; the
 * terminal boundary carries `terminal = 1` (its bootstrap is `{}`).
 * `templateFns` (condition closures) are not part of the JSON envelope —
 * the server writes them as an executable `<script>` between the envelope
 * script and the sentinel, so they are already on `window.__webui` by the
 * time the sentinel's `connectedCallback` runs.
 *
 * The terminal record may be markerless: it exists purely to close the
 * lifecycle and, when needed, resync a static tail that cannot contain
 * interactive roots. Every nonterminal boundary is compiler-authored and
 * therefore must have its complete marker pair.
 *
 * ## Two races, one seam
 *
 * Custom-element upgrade timing and boundary delivery are independent, so
 * either can arrive first:
 *
 * - **Runtime before boundary**: the authored element's class is already
 *   defined when its SSR markup parses, so `connectedCallback` runs before
 *   this coordinator has committed (or even seen) its boundary. Template
 *   metadata is missing entirely — `template-element.ts` defers quietly
 *   (`$deferredSSR = true`) instead of warning.
 * - **Boundary before runtime**: this coordinator commits a boundary whose
 *   elements aren't upgraded yet (their class isn't defined). Defining and
 *   walking the sentinel below still processes elements that are already
 *   upgraded; a still-undefined element is activated once its class is
 *   defined via `customElements.whenDefined` (allocated only on this
 *   uncommon path).
 *
 * Both paths converge on the existing `$deferredSSR` / `$activateDeferredSSR`
 * seam in `template-element.ts` — this module never mounts a component
 * directly.
 *
 * ## Single queue, single pump
 *
 * Sentinels enqueue themselves (cheap) in `connectedCallback` and return
 * immediately; a single microtask drains the queue, which guarantees that
 * boundary N's scaffolding (markers/script/sentinel) is fully removed
 * before boundary N+1 is ever processed. That removes the need to skip
 * "foreign" marker pairs while locating a boundary's own markers.
 */

import { registerTemplateData } from './template.js';
import type { TemplateMeta } from './template.js';
import { isStreamingHydrationMode } from './streaming-mode.js';
import {
  beginStreamingGate,
  abortStreamingGate,
  markBoundaryPending,
  markBoundaryCommitted,
  markLateActivationPending,
  settleLateActivation,
} from './lifecycle.js';
import { installTemplateElementRuntime } from './static-host.js';

/** Same registry key as `template-element.ts`'s `STREAMING_BOUNDARY_ACTIVATE`.
 *  `Symbol.for` resolves both sides to the identical runtime symbol without
 *  either module importing the other. */
const STREAMING_BOUNDARY_ACTIVATE = Symbol.for('microsoft.webui.boundaryActivate');

type BoundaryActivatable = Element & {
  [STREAMING_BOUNDARY_ACTIVATE]?: (state?: Record<string, unknown>) => void;
};

const SENTINEL_TAG = 'webui-hydrate';
const BOUNDARY_SCRIPT_ATTR = 'data-webui-boundary';
/** Explicit streamed-SSR-host marker emitted by the server/parser on every
 *  streamed component host root (and only those). It is the sole signal
 *  `template-element.ts` uses to defer a parser-created root until its boundary
 *  commits; the coordinator strips it from any root it abandons (reject,
 *  truncation, overflow) so a discarded boundary leaves no root stuck deferred. */
const STREAMED_HOST_ATTR = 'data-ws';
const BOUNDARY_START_PREFIX = 'wb:';
const BOUNDARY_END_PREFIX = '/wb:';
const SUPPORTED_VERSION = 1;
const BOUNDARY_HYDRATED_EVENT = 'webui:boundary-hydrated';

// Bounded work per boundary — a malformed or hostile stream cannot make this
// coordinator do unbounded work or retain unbounded memory.
const MAX_BOUNDARY_PAYLOAD_CHARS = 2_000_000;
const MAX_TEMPLATES_PER_BOUNDARY = 500;
// Counts every walked element (not just custom-element roots) — the walk
// itself, not just activation, is what this bounds.
const MAX_ELEMENTS_PER_BOUNDARY = 10_000;
const MAX_QUEUED_BOUNDARIES = 512;
const MAX_MARKER_SCAN_NODES = 50_000;
const MAX_BOUNDARY_SCRIPT_SCAN = 8;
const MAX_PENDING_UNDEFINED_ROOTS = 50_000;

/** Bootstrap payload shape — matches `window.__webui`, minus `templateFns`
 *  (installed directly by the server's executable extension script). */
export interface BoundaryBootstrap {
  state?: Record<string, unknown>;
  templates?: Record<string, TemplateMeta>;
  inventory?: string;
  nonce?: string;
  chain?: unknown[];
  css?: string[];
  styles?: string[];
  [key: string]: unknown;
}

export type BoundaryEnvelope = readonly [
  version: number,
  sequence: number,
  terminal: number,
  bootstrap: BoundaryBootstrap,
];

export type ParseBoundaryEnvelopeResult =
  | { readonly ok: true; readonly envelope: BoundaryEnvelope }
  | { readonly ok: false; readonly reason: string };

/**
 * Parse and structurally validate one boundary envelope's JSON text.
 *
 * Pure and side-effect free so it can be unit tested directly. Does not (and
 * cannot) validate sequence ordering against prior boundaries — that is a
 * property of the stream, checked by `processSentinel`.
 */
export function parseBoundaryEnvelope(text: string): ParseBoundaryEnvelopeResult {
  if (text.length > MAX_BOUNDARY_PAYLOAD_CHARS) {
    return { ok: false, reason: `boundary payload exceeds ${MAX_BOUNDARY_PAYLOAD_CHARS} characters` };
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return { ok: false, reason: 'boundary payload is not valid JSON' };
  }

  if (!Array.isArray(parsed) || parsed.length !== 4) {
    return { ok: false, reason: 'boundary envelope must be a 4-element [version, sequence, terminal, bootstrap] array' };
  }

  const version: unknown = parsed[0];
  const sequence: unknown = parsed[1];
  const terminal: unknown = parsed[2];
  const bootstrap: unknown = parsed[3];

  if (version !== SUPPORTED_VERSION) {
    return { ok: false, reason: `unsupported boundary envelope version ${JSON.stringify(version)}` };
  }
  if (typeof sequence !== 'number' || !Number.isInteger(sequence) || sequence < 0) {
    return { ok: false, reason: 'boundary sequence must be a non-negative integer' };
  }
  if (typeof terminal !== 'number' || (terminal !== 0 && terminal !== 1)) {
    return { ok: false, reason: 'boundary terminal flag must be 0 or 1' };
  }
  if (typeof bootstrap !== 'object' || bootstrap === null || Array.isArray(bootstrap)) {
    return { ok: false, reason: 'boundary bootstrap must be an object' };
  }

  const templates = (bootstrap as BoundaryBootstrap).templates;
  if (templates && Object.keys(templates).length > MAX_TEMPLATES_PER_BOUNDARY) {
    return { ok: false, reason: `boundary declares more than ${MAX_TEMPLATES_PER_BOUNDARY} templates` };
  }

  // Use the SUPPORTED_VERSION constant rather than the parsed `version`
  // value: they are equal at this point (checked above), and this sidesteps
  // relying on equality-narrowing an `unknown` down to a literal type.
  return { ok: true, envelope: [SUPPORTED_VERSION, sequence, terminal, bootstrap as BoundaryBootstrap] };
}

// ── Sentinel custom element ──────────────────────────────────────────

class WebUiHydrateSentinel extends HTMLElement {
  connectedCallback(): void {
    enqueueSentinel(this);
  }
}

// ── Single FIFO queue + microtask pump ───────────────────────────────

const queue: Element[] = [];
let queueHead = 0;
let pumpScheduled = false;

interface PendingTagWaiter {
  readonly generation: number;
  readonly roots: Set<Element>;
}

/** Exact roots whose class was still undefined when their boundary committed,
 *  keyed by tag. One `customElements.whenDefined` waiter is registered per tag
 *  (never per instance), while the bounded root sets keep detached elements and
 *  roots spread across several boundaries from being lost to a document query. */
const pendingTagWaiters = new Map<string, PendingTagWaiter>();
let pendingUndefinedRoots = 0;

/** Per-element stash of the boundary `state` a deferred root must see when it
 *  is finally activated (its own boundary has long since committed and later
 *  boundaries never publish their state globally). Held on the element itself,
 *  while the bounded per-tag set retains only the exact element reference.
 *  Property *presence* records that a root was stashed even when its boundary
 *  carried no state — see `NO_BOUNDARY_STATE`. */
const PENDING_BOUNDARY_STATE = Symbol('microsoft.webui.streaming.pendingState');

/** Stored in place of an `undefined` boundary state so a stashed root that
 *  carried no state is still distinguishable (by property presence) from one
 *  that was never stashed — without allocating a `{ state }` wrapper per root. */
const NO_BOUNDARY_STATE: unique symbol = Symbol('microsoft.webui.streaming.noState');

/** Shared reconnect seam installed only on roots that were undefined at
 *  checkpoint time. A detached root is not upgraded by `define()`; when it is
 *  later attached, `TemplateElement.connectedCallback()` invokes this shared
 *  function after entering its deferred state. No per-root closure is needed. */
const PENDING_ROOT_CONNECTED = Symbol.for('microsoft.webui.pendingRootConnected');

type PendingRoot = Element & {
  [PENDING_ROOT_CONNECTED]?: () => void;
};

/** Once true, a malformed/out-of-order envelope was seen — stop processing.
 *  The stream's ordering contract is broken, so nothing downstream can be
 *  trusted; halting (rather than skipping) avoids silently losing state. */
let halted = false;
let nextExpectedSequence = 0;

/** Set once a boundary carrying the terminal record commits successfully. A
 *  fully parsed response cannot legally emit another parser-inserted sentinel
 *  after its terminal boundary, so any sentinel seen afterward is malformed
 *  and is rejected rather than committed. Checked once per sentinel on the
 *  hot path (a single local boolean read). */
let terminalCommitted = false;

/** A terminal boundary remains lifecycle-pending for one microtask after its
 *  queue drain. This lets already-queued sentinel reactions append records
 *  behind it; an illegal post-terminal record then aborts before completion. */
let pendingTerminalSequence: number | null = null;
let terminalValidationScheduled = false;

/** Monotonic generation for the whole coordinator. `whenDefined` promise
 *  reactions are uncancellable, so each waiter captures the generation it was
 *  registered in; a reaction that resolves after the coordinator was reset
 *  (only `__resetStreamingCoordinatorForTests` bumps this) belongs to a stale
 *  generation and must not touch current state or the lifecycle counters.
 *  Always `0` in production, where the coordinator is never reset. */
let coordinatorGeneration = 0;

/** Remove a node from the DOM without letting a hostile/detached node throw. */
function safeRemove(node: Node | null | undefined): void {
  try {
    (node as ChildNode | null | undefined)?.remove?.();
  } catch {
    /* already detached or non-removable — nothing to bound */
  }
}

/** Remove an attribute without letting a hostile/exotic element throw. */
function safeRemoveAttribute(el: Element, name: string): void {
  try {
    el.removeAttribute?.(name);
  } catch {
    /* non-removable attribute host — nothing to bound */
  }
}

function enqueueSentinel(sentinel: Element): void {
  // Once halted, or once the pending queue is saturated by a hostile stream,
  // the boundary cannot be processed — discard its whole scaffold (sentinel,
  // payload/extension scripts, and any discoverable markers) so a rejected
  // boundary leaves nothing live behind, while its SSR roots stay in place.
  if (halted) {
    discardRejectedBoundary(sentinel);
    return;
  }
  if (queue.length - queueHead >= MAX_QUEUED_BOUNDARIES) {
    // A saturated queue means the stream is producing boundaries faster than
    // they can be trusted/drained — treat it as untrustworthy: clean this
    // boundary's scaffold and halt, which also sweeps every queued boundary.
    failBoundary(sentinel, `queued boundary count exceeds ${MAX_QUEUED_BOUNDARIES}`);
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
  // Drop the backing array once fully drained instead of letting it grow
  // across the page's whole streaming lifetime.
  queue.length = 0;
  queueHead = 0;
  scheduleTerminalValidation();
}

function fail(reason: string): void {
  halted = true;
  console.error(`[WebUI] streaming hydration halted: ${reason}.`);
  // Abort the completion gate *before* abandoning waiters: abandonment settles
  // each outstanding late-activation, which would otherwise drive the pending
  // counters to zero and dispatch `webui:hydration-complete` for a stream that
  // never legitimately finished (e.g. a malformed record arriving after a
  // committed terminal that still had an undefined-tag waiter open).
  abortStreamingGate();
  // Bounded abandonment of any still-pending undefined-tag waiters: strip the
  // stashed state from their live elements and balance each outstanding
  // late-activation exactly once. Their promises are uncancellable, but
  // `onTagDefined` no-ops for an abandoned waiter, so a later resolution
  // cannot double-count.
  abandonPendingWaiters();
  // A tentatively committed terminal keeps one boundary lifecycle count open
  // until queue validation. Settle it only after aborting the gate so failure
  // can never dispatch completion.
  settlePendingTerminal(false);
  // Bounded cleanup: discard any queued-but-unprocessed boundaries' scaffolding
  // so the coordinator retains nothing once the stream is declared untrustworthy.
  for (let i = queueHead; i < queue.length; i++) discardRejectedBoundary(queue[i]);
  queue.length = 0;
  queueHead = 0;
}

/** Halt the stream because of a specific sentinel, discarding that sentinel's
 *  own scaffold first (it has been dequeued, so `fail`'s queue sweep won't see
 *  it) and then abandoning everything else. */
function failBoundary(sentinel: Element, reason: string): void {
  discardRejectedBoundary(sentinel);
  fail(reason);
}

/**
 * Structurally discover and remove a rejected boundary's scaffold — its
 * sentinel, payload/extension scripts, and marker pair — without a parsed
 * envelope, leaving any SSR roots in place. Bounded and throw-safe: used on
 * every reject/halt/overflow path so a discarded boundary leaves zero
 * discoverable scaffold behind.
 */
function discardRejectedBoundary(sentinel: Element): void {
  const scriptEl = findBoundaryScript(sentinel);
  let endMarker: Comment | null = null;
  let startMarker: Comment | null = null;
  if (scriptEl) {
    endMarker = findEndMarkerByPrefix(scriptEl);
    if (endMarker) startMarker = findStartMarkerByPrefix(endMarker);
  }
  // Strip the streamed-host marker from this boundary's roots *before* removing
  // the markers that delimit them, so a rejected root is not left stuck in the
  // deferred state (a later class definition then mounts it normally).
  if (startMarker && endMarker) stripDeferredRootMarkers(startMarker, endMarker);
  removeBoundaryScaffolding(sentinel, scriptEl, startMarker, endMarker);
}

/**
 * Strip the `data-ws` streamed-host marker from every element between a
 * rejected boundary's markers. Single bounded pre-order walk that stops at the
 * exact end marker and allocates no list; the SSR roots themselves stay in the
 * tree. Used on reject/halt paths so a discarded boundary leaves no root stuck
 * deferred behind its (now-removed) markers.
 */
function stripDeferredRootMarkers(startMarker: Comment, endMarker: Node): void {
  if (!startMarker.parentNode) return;
  let node: Node | null = startMarker.nextSibling;
  let visited = 0;
  while (node && node !== endMarker) {
    if (visited++ >= MAX_MARKER_SCAN_NODES) break;
    if (node.nodeType === 1 /* ELEMENT_NODE */) safeRemoveAttribute(node as Element, STREAMED_HOST_ATTR);
    node = nextInBoundaryOrder(node);
  }
}

/** Balance and clear every pending undefined-tag waiter exactly once. */
function abandonPendingWaiters(): void {
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

/** Drop all coordinator-owned state from one abandoned undefined root. */
function clearPendingRoot(el: Element): void {
  if (hasPendingState(el)) takePendingState(el);
  delete (el as PendingRoot)[PENDING_ROOT_CONNECTED];
  safeRemoveAttribute(el, STREAMED_HOST_ATTR);
}

// ── Per-boundary processing ──────────────────────────────────────────

function processSentinel(sentinel: Element): void {
  // A well-formed, fully parsed response never emits another sentinel after
  // its terminal boundary. One arriving here means a corrupted/injected tail:
  // reject it (clean its scaffold) and halt rather than commit it, without
  // disturbing the successful completion the terminal boundary already drove.
  if (terminalCommitted) {
    failBoundary(sentinel, 'boundary arrived after the terminal streaming record');
    return;
  }

  const scriptEl = findBoundaryScript(sentinel);
  if (!scriptEl) {
    failBoundary(sentinel, 'missing boundary payload script before <webui-hydrate>');
    return;
  }

  const parsed = parseBoundaryEnvelope(scriptEl.textContent ?? '');
  if (!parsed.ok) {
    failBoundary(sentinel, parsed.reason);
    return;
  }

  const [, sequence, terminal, bootstrap] = parsed.envelope;
  if (sequence !== nextExpectedSequence) {
    failBoundary(sentinel, `expected boundary sequence ${nextExpectedSequence}, received ${sequence}`);
    return;
  }

  const resolved = resolveBoundaryRange(scriptEl, sequence, terminal === 1);
  if (!resolved.ok) {
    if (resolved.truncated) {
      // The placement strategy located the orphaned roots, so release them
      // directly rather than re-deriving a scaffold it already knows is broken.
      if (resolved.start) stripDeferredRootMarkers(resolved.start, scriptEl);
      removeBoundaryScaffolding(sentinel, scriptEl, resolved.start, null);
      fail(resolved.reason);
    } else {
      failBoundary(sentinel, resolved.reason);
    }
    return;
  }

  nextExpectedSequence++;
  commitBoundary(bootstrap, resolved.range, sequence, terminal === 1, sentinel, scriptEl);
}

/**
 * The DOM span one committed boundary activates.
 *
 * `start` and `end` are `null` together for a markerless record — an implicit
 * tail resync or a terminal record that introduced no new SSR roots.
 */
interface HydrationRange {
  readonly start: Comment | null;
  readonly end: Comment | null;
}

/** Shared immutable range for records that activate no new SSR roots. */
const MARKERLESS_RANGE: HydrationRange = { start: null, end: null };

/** Outcome of locating a boundary's activation span. */
type RangeResolution =
  | { readonly ok: true; readonly range: HydrationRange }
  /** Malformed in a way the generic structural reject path can clean up. */
  | { readonly ok: false; readonly reason: string; readonly truncated: false }
  /**
   * The response was cut between a boundary's start marker and its end marker,
   * so the exact scaffold is known here but not rediscoverable structurally.
   * `start` is the orphaned start marker when one was found.
   */
  | { readonly ok: false; readonly reason: string; readonly truncated: true; readonly start: Comment | null };

/**
 * Resolve where boundary `sequence` should hydrate.
 *
 * This is the coordinator's **only** placement-aware step. Everything after it
 * — template registration, state seeding, activation, scaffolding removal,
 * lifecycle accounting — operates on an abstract {@link HydrationRange} and
 * never inspects DOM adjacency. Native in-order streaming resolves a range from
 * the comment marker pair that brackets the payload; a future declarative
 * partial-updates transport can resolve one from a fragment target instead,
 * without forking the hydration lifecycle.
 *
 * The returned record is local to one commit and becomes garbage as soon as
 * `commitBoundary` returns, so it never contributes to retained heap. Its cost
 * is one short-lived object per boundary — boundaries are O(3-5) per response
 * and each already allocates a parsed envelope, so this is unmeasurable.
 */
function resolveBoundaryRange(scriptEl: Element, sequence: number, terminal: boolean): RangeResolution {
  // A boundary that introduces no new SSR roots (an implicit tail resync or
  // the terminal record) has no marker pair at all — only look for a start
  // marker once an end marker is actually present.
  const end = findEndMarker(scriptEl, sequence);
  if (end) {
    const start = findStartMarkerBefore(end, sequence);
    if (!start) {
      return { ok: false, reason: `missing start marker for boundary ${sequence}`, truncated: false };
    }
    return { ok: true, range: { start, end } };
  }
  if (terminal) return { ok: true, range: MARKERLESS_RANGE };
  // Only a terminal record may be markerless. Recover an orphaned exact start
  // marker so rejection can also release roots that would otherwise retain
  // their data-ws deferral forever.
  return {
    ok: false,
    reason: `missing end marker for boundary ${sequence}`,
    truncated: true,
    start: findStartMarkerBefore(scriptEl, sequence),
  };
}

function findBoundaryScript(sentinel: Element): Element | null {
  let node: Element | null = sentinel.previousElementSibling;
  for (let hops = 0; node && hops < MAX_BOUNDARY_SCRIPT_SCAN; hops++) {
    if (node.tagName === 'SCRIPT' && node.hasAttribute(BOUNDARY_SCRIPT_ATTR)) return node;
    node = node.previousElementSibling;
  }
  return null;
}

function findEndMarker(scriptEl: Element, sequence: number): Comment | null {
  const node = scriptEl.previousSibling;
  if (node && node.nodeType === 8 /* COMMENT_NODE */ && (node as Comment).data === `${BOUNDARY_END_PREFIX}${sequence}`) {
    return node as Comment;
  }
  return null;
}

function findStartMarkerBefore(nodeBefore: Node, sequence: number): Comment | null {
  const target = `${BOUNDARY_START_PREFIX}${sequence}`;
  let node: Node | null = nodeBefore.previousSibling;
  for (let hops = 0; node && hops < MAX_MARKER_SCAN_NODES; hops++) {
    if (node.nodeType === 8 /* COMMENT_NODE */ && (node as Comment).data === target) {
      return node as Comment;
    }
    node = node.previousSibling;
  }
  return null;
}

/** End-marker discovery for the reject path, where no valid sequence was
 *  parsed: match structurally on the `/wb:` prefix rather than an exact
 *  sequence, so a malformed boundary's markers are still discoverable. */
function findEndMarkerByPrefix(scriptEl: Element): Comment | null {
  const node = scriptEl.previousSibling;
  if (node && node.nodeType === 8 /* COMMENT_NODE */ && (node as Comment).data.startsWith(BOUNDARY_END_PREFIX)) {
    return node as Comment;
  }
  return null;
}

/** Paired start-marker discovery for the reject path: the end marker's data is
 *  `/wb:<seq>`, so match the exact `wb:<seq>` start marker (never a foreign
 *  boundary's marker) by reusing the sequence embedded in the end marker. */
function findStartMarkerByPrefix(endMarker: Comment): Comment | null {
  const target = `${BOUNDARY_START_PREFIX}${endMarker.data.slice(BOUNDARY_END_PREFIX.length)}`;
  let node: Node | null = endMarker.previousSibling;
  for (let hops = 0; node && hops < MAX_MARKER_SCAN_NODES; hops++) {
    if (node.nodeType === 8 /* COMMENT_NODE */ && (node as Comment).data === target) {
      return node as Comment;
    }
    node = node.previousSibling;
  }
  return null;
}

/**
 * Install this boundary's templates/state and, if it introduced any new
 * SSR roots, activate every deferred component in its marker range, then
 * release lifecycle bookkeeping.
 *
 * Accounting is throw-safe: `markBoundaryPending()`/`markBoundaryCommitted()`
 * always balance, even if template registration or global handoff throws, so
 * a failure can never wedge `webui:hydration-complete` on a stuck pending
 * count. Scaffolding and the transient payload are removed *before* any
 * boundary/completion event fires. Individual component activation failures
 * are isolated inside `activateRootsBetween` and do **not** halt the stream,
 * but exhausting a bounded range/root limit is an unrecoverable stream error.
 *
 * `bootstrap` is a local parameter and the marker range is walked exactly
 * once. Defined roots retain nothing; only the uncommon undefined-root path
 * keeps bounded exact element/state references until definition or halt.
 *
 * Placement is already resolved: this function consumes an abstract
 * {@link HydrationRange} and never inspects DOM adjacency itself.
 */
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
    // registerTemplateData() can throw for a compiled condition-function
    // reference that doesn't resolve (a template/build-integrity bug) — an
    // unrecoverable error routed through the same halt path, unlike a single
    // component's activation failure, which is isolated below.
    if (bootstrap.templates) registerTemplateData(bootstrap.templates);
    applyBootstrapGlobals(bootstrap);
    if (range.start && range.end) activateRootsBetween(range.start, range.end, bootstrap.state);
    committed = true;
  } catch (error) {
    fail(`error committing boundary ${sequence}: ${error instanceof Error ? error.message : String(error)}`);
  } finally {
    // Remove this boundary's scaffolding and drop the transient payload
    // before any event fires, and always settle the pending-boundary count.
    // A failed commit still settles its pending boundary, but must NOT mark
    // the terminal record reached: otherwise a stream that threw on its
    // terminal boundary could dispatch `webui:hydration-complete` as if it
    // had succeeded.
    removeBoundaryScaffolding(sentinel, scriptEl, range.start, range.end);
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

/**
 * Merge this boundary's bootstrap into the global `window.__webui` handoff,
 * preserving accumulated templates (merged separately via
 * `registerTemplateData`) and skipping the ephemeral per-boundary `state`
 * (handed directly to each activating root) rather than overwriting them.
 */
function applyBootstrapGlobals(bootstrap: BoundaryBootstrap): void {
  const w = window as Window;
  if (!w.__webui) w.__webui = {};
  const keys = Object.keys(bootstrap);
  for (let i = 0; i < keys.length; i++) {
    const key = keys[i];
    // Templates were merged above via registerTemplateData; overwriting
    // window.__webui.templates wholesale here would drop earlier boundaries'
    // templates that this boundary doesn't happen to repeat.
    if (key === 'templates') continue;
    // Boundary `state` is ephemeral and per-boundary — it is handed straight
    // to each activating root (see `invokeActivationHook`) and never published
    // on the global handoff. Publishing it would let a later boundary clobber
    // an earlier one's activation state and leave committed boundary state
    // retained globally for the page's lifetime.
    if (key === 'state') continue;
    if (key === 'inventory') {
      w.__webui.inventory = mergeInventory(w.__webui.inventory, bootstrap.inventory);
      continue;
    }
    // The streaming writer emits CSS/style bookkeeping only for tags first
    // seen in this checkpoint. Preserve all prior deltas and avoid duplicates.
    if (key === 'css' || key === 'styles') {
      appendUniqueStrings(w.__webui, key, bootstrap[key]);
      continue;
    }
    w.__webui[key] = bootstrap[key];
  }
}

/**
 * OR two compact inventory byte strings without changing their byte order.
 *
 * The server serializes byte zero first and removes only trailing zero bytes,
 * so shorter values are padded on the right, not numerically left-padded.
 */
function mergeInventory(existing: unknown, delta: unknown): string {
  validateInventory(existing, 'existing');
  validateInventory(delta, 'boundary');
  const current = existing ?? '';
  const next = delta ?? '';
  const length = Math.max(current.length, next.length);
  let merged = '';
  for (let i = 0; i < length; i++) {
    const a = i < current.length ? Number.parseInt(current[i], 16) : 0;
    const b = i < next.length ? Number.parseInt(next[i], 16) : 0;
    merged += (a | b).toString(16);
  }
  return merged;
}

function validateInventory(value: unknown, source: string): asserts value is string | undefined {
  if (value === undefined) return;
  if (typeof value !== 'string' || value.length % 2 !== 0) {
    throw new Error(`${source} inventory must be an even-length hexadecimal string`);
  }
  for (let i = 0; i < value.length; i++) {
    const code = value.charCodeAt(i);
    const isDigit = code >= 48 && code <= 57;
    const isUpperHex = code >= 65 && code <= 70;
    const isLowerHex = code >= 97 && code <= 102;
    if (!isDigit && !isUpperHex && !isLowerHex) {
      throw new Error(`${source} inventory must be an even-length hexadecimal string`);
    }
  }
}

function appendUniqueStrings(target: NonNullable<Window['__webui']>, key: 'css' | 'styles', delta: unknown): void {
  if (!Array.isArray(delta) || !delta.every((value) => typeof value === 'string')) {
    throw new Error(`boundary ${key} must be an array of strings`);
  }
  const existing = target[key];
  if (existing !== undefined && (!Array.isArray(existing) || !existing.every((value) => typeof value === 'string'))) {
    throw new Error(`existing ${key} must be an array of strings`);
  }
  const values = existing as string[] | undefined;
  const cumulative = values ?? [];
  for (let i = 0; i < delta.length; i++) {
    if (cumulative.indexOf(delta[i]) === -1) cumulative.push(delta[i]);
  }
  if (!values) target[key] = cumulative;
}

/**
 * Walk every element between `startMarker` and `endMarker` (exclusive) in one
 * pre-order pass, including open declarative shadow roots, and activate any
 * deferred custom element found.
 *
 * The walk advances in boundary order and stops the instant it reaches the
 * exact end-marker comment node, so it needs no per-element
 * `compareDocumentPosition` bounds check and allocates no intermediate list of
 * roots — it visits each node once and returns. A single flat walk (rather
 * than a per-root descendant scan) is also what avoids double-activating a
 * nested component. Each root's activation is isolated: a component that
 * throws while mounting is logged and skipped so later roots — and later
 * boundaries — still hydrate.
 */
function activateRootsBetween(
  startMarker: Comment,
  endMarker: Comment,
  state: Record<string, unknown> | undefined,
): void {
  if (!startMarker.parentNode) throw new Error('boundary start marker was detached before activation');

  let node: Node | null = startMarker.nextSibling;
  let elements = 0;
  let visited = 0;
  let failure: string | null = null;
  while (node && node !== endMarker) {
    // Defensive bound against adversarial mid-flight DOM mutation that could
    // detach `endMarker` from the forward path; a well-formed boundary always
    // reaches `endMarker` well within this cap.
    if (visited >= MAX_MARKER_SCAN_NODES) {
      failure = `streaming boundary walk exceeds ${MAX_MARKER_SCAN_NODES} nodes`;
      break;
    }
    visited++;
    if (node.nodeType === 1 /* ELEMENT_NODE */) {
      if (elements >= MAX_ELEMENTS_PER_BOUNDARY) {
        failure = `streaming boundary exceeds ${MAX_ELEMENTS_PER_BOUNDARY} elements`;
        break;
      }
      elements++;
      try {
        const activationFailure = activateElement(node as Element, state);
        if (activationFailure) {
          failure = activationFailure;
          break;
        }
      } catch (error) {
        // Isolate one component's activation failure — the stream is still
        // well-formed, so later roots and boundaries must continue.
        console.error(`[WebUI] streaming: activation failed for <${(node as Element).tagName?.toLowerCase?.() ?? '?'}>: ${error instanceof Error ? error.message : String(error)}`);
      }
    }
    node = nextInBoundaryOrder(node);
  }
  if (!failure && node !== endMarker) failure = 'streaming boundary end marker became unreachable during activation';
  if (!failure) return;

  // Continue forward from the first unprocessed node, stripping streamed-host
  // markers while work remains bounded. This is not a rescan: every range node
  // is visited at most once by this commit.
  let cleanupVisited = 0;
  while (node && node !== endMarker && cleanupVisited < MAX_MARKER_SCAN_NODES) {
    if (node.nodeType === 1 /* ELEMENT_NODE */) {
      safeRemoveAttribute(node as Element, STREAMED_HOST_ATTR);
    }
    cleanupVisited++;
    node = nextInBoundaryOrder(node);
  }
  throw new Error(failure);
}

/**
 * Next node in allocation-free boundary pre-order.
 *
 * An open declarative shadow root is traversed before the host's light-DOM
 * children. When its subtree ends, the walk resumes at those light children
 * and then at the host's sibling, so every reachable node is visited once.
 */
function nextInBoundaryOrder(node: Node): Node | null {
  if (node.nodeType === 1 /* ELEMENT_NODE */) {
    const shadowRoot = (node as Element).shadowRoot;
    if (shadowRoot?.firstChild) return shadowRoot.firstChild;
  }
  if (node.firstChild) return node.firstChild;
  let current: Node | null = node;
  while (current) {
    if (current.nodeType === 11 /* DOCUMENT_FRAGMENT_NODE */) {
      const host: Element | null = (current as ShadowRoot).host ?? null;
      if (host) {
        if (host.firstChild) return host.firstChild;
        current = host;
        continue;
      }
    }
    if (current.nextSibling) return current.nextSibling;
    current = current.parentNode;
  }
  return null;
}

function activateElement(el: Element, state: Record<string, unknown> | undefined): string | null {
  if (!el.hasAttribute(STREAMED_HOST_ATTR)) return null;
  const tag = el.tagName.toLowerCase();
  if (tag.indexOf('-') === -1) return null;

  if (customElements.get(tag)) {
    invokeActivationHook(el, state);
    return null;
  }

  // Uncommon path: the element's class isn't defined yet even though its
  // boundary already committed. Track the exact root in a bounded per-tag set
  // and register at most one native waiter per tag. Exact tracking is required:
  // a document query loses detached roots, and a one-shot query cap strands
  // roots accumulated across several valid boundaries.
  if (!hasPendingState(el) && pendingUndefinedRoots >= MAX_PENDING_UNDEFINED_ROOTS) {
    return `pending undefined root count exceeds ${MAX_PENDING_UNDEFINED_ROOTS}`;
  }
  stashPendingState(el, state);
  let waiter = pendingTagWaiters.get(tag);
  if (!waiter) {
    waiter = { generation: coordinatorGeneration, roots: new Set() };
    pendingTagWaiters.set(tag, waiter);
    // The boundary has committed but these roots are still inert; keep the
    // completion gate open until this wait resolves.
    markLateActivationPending();
    // Capture the current generation so a stale resolution (after a reset)
    // is ignored instead of settling the wrong generation's counter.
    const generation = coordinatorGeneration;
    customElements.whenDefined(tag).then(() => onTagDefined(tag, generation));
  }
  if (!waiter.roots.has(el)) {
    waiter.roots.add(el);
    pendingUndefinedRoots++;
  }
  (el as PendingRoot)[PENDING_ROOT_CONNECTED] = resumePendingRoot;
  return null;
}

/**
 * Activate every still-deferred root of `tag` once its class is defined.
 *
 * `define()` upgrades connected roots before resolving `whenDefined`. Detached
 * exact roots are upgraded explicitly, then activated without requiring
 * attachment. `PENDING_ROOT_CONNECTED` still handles the synchronous race where
 * a root attaches and upgrades before this promise reaction runs.
 */
function onTagDefined(tag: string, generation: number): void {
  if (generation !== coordinatorGeneration) return;
  const waiter = pendingTagWaiters.get(tag);
  if (!waiter || waiter.generation !== generation) return;
  for (const el of waiter.roots) {
    if (!el.isConnected) {
      try {
        customElements.upgrade(el);
      } catch (error) {
        fail(`failed to upgrade detached <${tag}>: ${error instanceof Error ? error.message : String(error)}`);
        return;
      }
      const hook = (el as BoundaryActivatable)[STREAMING_BOUNDARY_ACTIVATE];
      if (el.hasAttribute(STREAMED_HOST_ATTR) && typeof hook !== 'function') {
        fail(`upgrading detached <${tag}> did not install its streaming activation hook`);
        return;
      }
    }
    try {
      activatePendingRoot(tag, waiter, el);
    } catch (error) {
      console.error(`[WebUI] streaming: late activation failed for <${tag}>: ${error instanceof Error ? error.message : String(error)}`);
    }
  }
}

/** Resume a root that was detached when its class became defined. Installed as
 *  one shared function value, so the uncommon path retains no per-root closure. */
function resumePendingRoot(this: Element): void {
  const tag = this.tagName.toLowerCase();
  const waiter = pendingTagWaiters.get(tag);
  if (!waiter || waiter.generation !== coordinatorGeneration || !waiter.roots.has(this)) {
    delete (this as PendingRoot)[PENDING_ROOT_CONNECTED];
    return;
  }
  activatePendingRoot(tag, waiter, this);
}

function activatePendingRoot(tag: string, waiter: PendingTagWaiter, el: Element): void {
  if (!waiter.roots.delete(el)) return;
  pendingUndefinedRoots--;
  delete (el as PendingRoot)[PENDING_ROOT_CONNECTED];
  const state = takePendingState(el);
  try {
    invokeActivationHook(el, state);
  } finally {
    if (waiter.roots.size === 0 && pendingTagWaiters.get(tag) === waiter) {
      pendingTagWaiters.delete(tag);
      settleLateActivation();
    }
  }
}

/** Stash the boundary state a deferred root must see when finally activated.
 *  Stores the state reference directly (or `NO_BOUNDARY_STATE` when the
 *  boundary carried none) so property presence still records the stash without
 *  allocating a per-root wrapper object. */
function stashPendingState(el: Element, state: Record<string, unknown> | undefined): void {
  (el as unknown as Record<symbol, unknown>)[PENDING_BOUNDARY_STATE] =
    state === undefined ? NO_BOUNDARY_STATE : state;
}

function hasPendingState(el: Element): boolean {
  return Object.prototype.hasOwnProperty.call(el, PENDING_BOUNDARY_STATE);
}

function takePendingState(el: Element): Record<string, unknown> | undefined {
  const store = el as unknown as Record<symbol, unknown>;
  const stored = store[PENDING_BOUNDARY_STATE];
  delete store[PENDING_BOUNDARY_STATE];
  return stored === NO_BOUNDARY_STATE ? undefined : (stored as Record<string, unknown> | undefined);
}

/**
 * Invoke a deferred root's activation hook with its boundary-local `state`,
 * then strip its compiler scaffolding marker.
 *
 * Boundary state is not cumulative: a late activation must see the
 * state that was live when its *own* boundary committed. The coordinator no
 * longer routes that through the global `window.__webui.state` (a later
 * boundary may have replaced it, and `applyBootstrapGlobals` deliberately
 * never publishes boundary state globally) — it hands the state straight to
 * the element's activation hook, which threads it into SSR-state application.
 *
 * The `finally` is the single authority for successful-path `data-ws` removal:
 * every committed root loses the streamed-host marker here — whether it has no
 * hook (a plain defined element), a hook that opts out (a compiler-owned static
 * host — see `static-host.ts`), or a hook that throws. Centralizing it in the
 * coordinator guarantees no committed root is ever left advertising itself as
 * still-streaming, so `TemplateElement` does not repeat the cleanup. A
 * non-streamed element never carries the attribute, so this is a no-op there.
 */
function invokeActivationHook(el: Element, state: Record<string, unknown> | undefined): void {
  try {
    const hook = (el as BoundaryActivatable)[STREAMING_BOUNDARY_ACTIVATE];
    if (typeof hook === 'function') hook.call(el, state);
  } finally {
    safeRemoveAttribute(el, STREAMED_HOST_ATTR);
  }
}

/** Remove this boundary's scaffolding: sentinel, boundary script (and any
 *  executable extension script between it and the sentinel), and markers
 *  (when this boundary introduced roots and therefore has markers). SSR roots
 *  sit *between* the start and end markers and are intentionally preserved —
 *  only the marker comments themselves, the payload/extension scripts, and the
 *  sentinel are removed, all of which sit *after* the roots (the scripts and
 *  sentinel) or bracket them (the markers). `scriptEl` may be null on the
 *  reject path when the payload script was not discoverable — then only the
 *  sentinel (and any markers passed in) are removed. Runs on both the commit
 *  finally and the reject paths, so every removal is throw-safe. */
function removeBoundaryScaffolding(
  sentinel: Element,
  scriptEl: Element | null,
  startMarker: Comment | null,
  endMarker: Comment | null,
): void {
  if (!scriptEl) {
    safeRemove(sentinel);
    safeRemove(endMarker);
    safeRemove(startMarker);
    return;
  }
  let node: Element | null = sentinel.previousElementSibling;
  safeRemove(sentinel);
  for (let hops = 0; node && hops <= MAX_BOUNDARY_SCRIPT_SCAN; hops++) {
    const previous: Element | null = node.previousElementSibling;
    const wasBoundaryScript = node === scriptEl;
    safeRemove(node);
    if (wasBoundaryScript) break;
    node = previous;
  }
  safeRemove(endMarker);
  safeRemove(startMarker);
}

/** Optional diagnostic/extension event, off by default. Allocating a
 *  `CustomEvent` per boundary is pure diagnostic overhead, so it is opt-in via
 *  a lightweight global flag (`window.__WEBUI_STREAMING_DEBUG__ = true`) set
 *  before page scripts run — e.g. by tests. `webui:hydration-complete` is the
 *  real completion signal and always fires regardless of this flag. */
function dispatchBoundaryHydrated(sequence: number, terminal: boolean): void {
  if (!boundaryEventsEnabled()) return;
  window.dispatchEvent(new CustomEvent(BOUNDARY_HYDRATED_EVENT, { detail: { sequence, terminal } }));
}

function boundaryEventsEnabled(): boolean {
  return (window as unknown as { __WEBUI_STREAMING_DEBUG__?: boolean }).__WEBUI_STREAMING_DEBUG__ === true;
}

function scheduleTerminalValidation(): void {
  if (pendingTerminalSequence === null || terminalValidationScheduled) return;
  terminalValidationScheduled = true;
  const generation = coordinatorGeneration;
  queueMicrotask(() => validatePendingTerminal(generation));
}

function validatePendingTerminal(generation: number): void {
  if (generation !== coordinatorGeneration) return;
  terminalValidationScheduled = false;
  if (pendingTerminalSequence === null) return;
  // A sentinel reaction already queued behind the terminal may have appended a
  // record and scheduled the pump. Let that pump validate/reject it first.
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

// ── Install ───────────────────────────────────────────────────────────

let installed = false;

/**
 * Install a one-shot `DOMContentLoaded` guard against a truncated response.
 *
 * A fully parsed streaming response always commits its terminal boundary
 * before the parser finishes, and cannot legally emit another parser-inserted
 * sentinel afterward. So if the document reaches `DOMContentLoaded` without a
 * successful terminal commit, the response was cut off mid-stream — route that
 * through the same bounded failure/abandonment path so no late-activation
 * waiter or stashed state is left live and `webui:hydration-complete` never
 * fires for a partial page.
 *
 * Exactly one guard is scheduled per install (never one per boundary): a
 * DOMContentLoaded listener while loading, or a microtask after it has already
 * fired. It captures the coordinator generation so a stale guard — only
 * possible after a test-time reset — no-ops instead of failing fresh state.
 */
function installTruncationGuard(): void {
  if (typeof document === 'undefined') return;
  const generation = coordinatorGeneration;
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', () => onDomContentLoaded(generation), { once: true });
  } else {
    // Install may run after DOMContentLoaded. Defer one microtask so sentinel
    // reactions already queued for a valid terminal record commit first.
    queueMicrotask(() => onDomContentLoaded(generation));
  }
}

function onDomContentLoaded(generation: number): void {
  if (generation !== coordinatorGeneration) return;
  if (halted || terminalCommitted) return;
  fail('response ended at DOMContentLoaded before the terminal streaming boundary committed (truncated stream)');
}

/**
 * Install the streaming coordinator for this document, if it was served in
 * streaming-hydration mode. Idempotent and cheap to call unconditionally —
 * a non-streaming page pays for exactly one cached meta-tag query.
 */
export function installStreamingCoordinator(): void {
  if (installed || !isStreamingHydrationMode()) return;
  installed = true;

  if (!customElements.get(SENTINEL_TAG)) customElements.define(SENTINEL_TAG, WebUiHydrateSentinel);
  beginStreamingGate();
  installTruncationGuard();
  // Compiler-owned dormant hosts must be claimable from templates that
  // arrive with the very first streamed boundary, not only after
  // DOMContentLoaded (see the matching fix in static-host.ts).
  installTemplateElementRuntime();
}

// ── Test-only surface ─────────────────────────────────────────────────
// The coordinator's queue/halt/sequence state and tag-waiter set are module
// singletons, so pipeline tests need a way to reset them between cases and to
// drive a sentinel through the pump without a real custom-element upgrade.
// These are internal, prefixed, and never referenced by production code.

/** Reset all coordinator singletons to their initial state. Balances any
 *  pending late-activation waiters exactly once (so the lifecycle count never
 *  underflows) and bumps the generation so an outstanding `whenDefined`
 *  reaction from a prior test no-ops instead of mutating fresh state. */
export function __resetStreamingCoordinatorForTests(): void {
  abandonPendingWaiters();
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
  installed = false;
}

/** Schedule the loading-state-aware truncation guard (as the coordinator
 *  install does), without the full meta-tag/custom-element install. */
export function __installTruncationGuardForTests(): void {
  installTruncationGuard();
}

/** Enqueue a sentinel exactly as `connectedCallback` would, driving the pump. */
export function __enqueueSentinelForTests(sentinel: Element): void {
  enqueueSentinel(sentinel);
}

/** Report whether the coordinator has halted (for assertions). */
export function __isHaltedForTests(): boolean {
  return halted;
}

/** Number of distinct undefined-tag waiters currently outstanding. */
export function __pendingTagWaiterCountForTests(): number {
  return pendingTagWaiters.size;
}

/** Number of exact undefined roots currently retained by tag waiters. */
export function __pendingUndefinedRootCountForTests(): number {
  return pendingUndefinedRoots;
}

/** Whether a given element still carries stashed pending boundary state. */
export function __elementHasPendingStateForTests(el: Element): boolean {
  return hasPendingState(el);
}
