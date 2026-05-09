// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Event delegation engine for WebUIElement.
 *
 * Two binding strategies:
 *
 * 1. **Global delegated events** — for bubbling-and-composed events, ONE
 *    listener is attached to `document` per event name (across the whole
 *    application). A WeakMap maps each registered target element to its
 *    handler entry. On dispatch, `event.composedPath()` is walked and each
 *    matched target's handler runs with its host as `this`.
 *
 *    Result: ~5 listeners total (click, input, keydown, etc.) instead of
 *    ~2-3 listeners per component (≈400+ on a list page). The browser
 *    auto-reclaims handler entries when the target element is GC'd, so no
 *    explicit cleanup is needed when shadow trees are removed.
 *
 * 2. **Direct events** — one listener per target, used for non-bubbling
 *    events (focus, blur, scroll) and root-level component events.
 *    Bindings are stored module-side in a WeakMap keyed by host;
 *    cleanup is explicit on host destruction.
 *
 * Destroyed hosts are tracked in a module-level WeakSet (`deadHosts`)
 * — the global dispatcher consults it before invoking handlers, so a
 * destroyed component can't accidentally receive late events through a
 * still-attached target.
 */

import type {
  TemplateBlockMeta,
  TemplateMeta,
  TemplateNodePath,
} from '../template.js';
import type { TemplateInstance } from './types.js';

// ── Types ────────────────────────────────────────────────────────

export type RuntimeEventHandler = (event: Event) => void;

interface HandlerEntry {
  host: EventStateHost;
  handler: RuntimeEventHandler;
}

/**
 * A direct (non-delegated) event listener binding.
 *
 * Implements the DOM `EventListener` interface via `handleEvent`, so the
 * binding object itself is registered with `addEventListener`.  This
 * eliminates the per-listener closure that `.bind(host)` would create —
 * a savings of one closure (~40B) per direct listener.
 */
export class DirectEventBinding {
  constructor(
    readonly target: EventTarget,
    readonly eventName: string,
    readonly host: EventStateHost,
    readonly method: RuntimeEventHandler,
    readonly owner?: TemplateInstance,
  ) {}

  handleEvent(event: Event): void {
    if (deadHosts.has(this.host)) return;
    this.method.call(this.host, event);
  }
}

/**
 * Host element shape required by the event engine.
 *
 * Direct (non-delegated) bindings and the kill-switch flag live in
 * module-level weak collections (`directBindings`, `deadHosts`)  rather
 * than as own properties on the host. This avoids ~16B per-host of
 * permanent slot overhead × hundreds of components per route.
 */
export interface EventStateHost extends EventTarget {
  [key: string]: unknown;
}

/**
 * Per-host direct (non-delegated) event listener bindings.
 *
 * Stored module-side rather than on the host so destroyed components
 * leave no slot footprint behind on instances that never registered a
 * direct listener.  WeakMap entries auto-release when the host is GC'd.
 */
const directBindings: WeakMap<EventStateHost, DirectEventBinding[]> =
  new WeakMap();

/**
 * Set of destroyed hosts.  Checked by the global dispatcher to drop
 * straggling events that fire through still-live target elements after
 * the host's `$destroy` has run.  WeakSet entries auto-release when the
 * host is GC'd, so the set stays small (only contains hosts whose
 * targets are still in the DOM but whose component has been disposed).
 */
const deadHosts: WeakSet<EventStateHost> = new WeakSet();

// ── Global delegation engine ─────────────────────────────────────

/**
 * One bucket per event name, all attached to `document`. Each bucket maps
 * targets to handler entries via a WeakMap so garbage-collected targets
 * release their entries automatically.
 */
class GlobalEventBucket {
  readonly targets: WeakMap<EventTarget, HandlerEntry | HandlerEntry[]>;

  constructor(readonly eventName: string) {
    this.targets = new WeakMap();
  }

  handleEvent(event: Event): void {
    const path = event.composedPath();
    if (path.length === 0) return;
    // When a `composed: true` event crosses shadow-DOM boundaries to reach
    // this document-level listener, `event.target` is retargeted to the
    // outermost shadow host. Handlers (e.g. `event.target instanceof
    // HTMLInputElement`) expect the original target inside the shadow root
    // — the first node in `composedPath()`. Restore it for the duration of
    // dispatch and revert when finished.
    const originalTarget = path[0];
    const retargeted = event.target !== originalTarget;
    if (retargeted) {
      Object.defineProperty(event, 'target', {
        value: originalTarget,
        configurable: true,
      });
    }
    try {
      for (let i = 0; i < path.length; i++) {
        const node = path[i];
        const entry = this.targets.get(node);
        if (entry === undefined) continue;
        invokeEntry(entry, event, node);
        if (event.cancelBubble) return;
      }
    } finally {
      if (retargeted) {
        delete (event as unknown as Record<string, unknown>)['target'];
      }
    }
  }
}

function invokeEntry(
  entry: HandlerEntry | HandlerEntry[],
  event: Event,
  node: EventTarget,
): void {
  Object.defineProperty(event, 'currentTarget', {
    value: node,
    configurable: true,
  });
  try {
    if (Array.isArray(entry)) {
      for (let j = 0; j < entry.length; j++) {
        const e = entry[j];
        if (deadHosts.has(e.host)) continue;
        e.handler.call(e.host, event);
        if (event.cancelBubble) return;
      }
    } else {
      if (deadHosts.has(entry.host)) return;
      entry.handler.call(entry.host, event);
    }
  } finally {
    delete (event as unknown as Record<string, unknown>)['currentTarget'];
  }
}

const globalBuckets = new Map<string, GlobalEventBucket>();

function getGlobalBucket(eventName: string): GlobalEventBucket {
  let bucket = globalBuckets.get(eventName);
  if (!bucket) {
    bucket = new GlobalEventBucket(eventName);
    document.addEventListener(eventName, bucket);
    globalBuckets.set(eventName, bucket);
  }
  return bucket;
}

// ── Pure helpers ─────────────────────────────────────────────────

/**
 * Returns true if `eventName` bubbles AND composes — i.e., crosses shadow
 * roots — so a single document-level listener can dispatch it via
 * `composedPath()`.
 *
 * Strategy:
 * - Hyphenated names (e.g. `input-change`, `toggle-nav-pane`) are custom
 *   events from web components. Convention dispatches them with
 *   `{bubbles: true, composed: true}`, so they always delegate.
 * - Standard DOM events use a small whitelist of events that satisfy
 *   `bubbles: true` AND `composed: true`. Events that bubble inside a
 *   shadow root but do not compose (notably `submit`, `reset`, `change`)
 *   are explicitly excluded — a `document`-level listener never sees them
 *   when fired from inside a component's shadow DOM. They fall through to
 *   the direct-listener code path instead.
 */
export function canDelegateEvent(eventName: string): boolean {
  if (eventName.indexOf('-') !== -1) return true;
  switch (eventName.toLowerCase()) {
    case 'beforeinput':
    case 'click':
    case 'contextmenu':
    case 'dblclick':
    case 'drag':
    case 'dragend':
    case 'dragenter':
    case 'dragleave':
    case 'dragover':
    case 'dragstart':
    case 'drop':
    case 'input':
    case 'keydown':
    case 'keypress':
    case 'keyup':
    case 'mousedown':
    case 'mousemove':
    case 'mouseout':
    case 'mouseover':
    case 'mouseup':
    case 'pointercancel':
    case 'pointerdown':
    case 'pointermove':
    case 'pointerout':
    case 'pointerover':
    case 'pointerup':
    case 'touchcancel':
    case 'touchend':
    case 'touchmove':
    case 'touchstart':
    case 'wheel':
      return true;
    // NOT delegated — `bubbles: true` but `composed: false`, so a
    // document-level listener never observes them when fired from inside
    // a component's shadow DOM. Handled via the direct-listener path.
    //   submit, reset, change
    default:
      return false;
  }
}

// ── Event wiring (called during $wire / $hydrate) ────────────────

/**
 * Wire compiled template events, root events, and `w-ref` attributes.
 *
 * Shared entry point used by both client-created ($wire) and SSR
 * hydration ($hydrate) paths.  The `resolver` parameter abstracts
 * the DOM lookup strategy (childNode index vs ordinal-based SSR walk).
 */
export function finalize(
  host: EventStateHost,
  root: Node,
  meta: TemplateBlockMeta,
  resolver: (root: Node, path: TemplateNodePath) => Node | null,
  owner: TemplateInstance,
): void {
  wireEvents(host, root, meta, resolver, owner);
  if ((meta as TemplateMeta).re) wireRootEvents(host, (meta as TemplateMeta).re!);
  wireRefs(root, owner, host);
}

/** Wire compiled element events using a resolver function. */
function wireEvents(
  host: EventStateHost,
  root: Node,
  meta: TemplateBlockMeta,
  resolver: (root: Node, path: TemplateNodePath) => Node | null,
  owner: TemplateInstance,
): void {
  if (!meta.e) return;
  for (let i = 0; i < meta.e.length; i++) {
    const [eventName, handlerName, needsEvent, target] = meta.e[i];
    const el = resolver(root, target);
    if (!el || el.nodeType !== 1) continue;
    addEvent(host, el as Element, eventName, handlerName, needsEvent, owner);
  }
}

/** Wire root-level events on the host element (or shadow root when present). */
function wireRootEvents(
  host: EventStateHost,
  re: [string, string, number][],
): void {
  const target = (host as unknown as HTMLElement).shadowRoot ?? host;
  for (let i = 0; i < re.length; i++) {
    addEvent(host, target, re[i][0], re[i][1], re[i][2], undefined);
  }
}

/** Find w-ref attributes and assign to component properties. */
function wireRefs(root: Node, owner: TemplateInstance, host: Record<string, unknown>): void {
  if (root.nodeType !== 1 && root.nodeType !== 11) return;
  const refs = (root as Element).querySelectorAll('[w-ref]');
  for (let i = 0; i < refs.length; i++) {
    const raw = refs[i].getAttribute('w-ref');
    if (!raw || raw.charCodeAt(0) !== 123) continue;
    const name = raw.slice(1, -1);
    if (name) {
      const node = refs[i] as Element;
      host[name] = node;
      owner.refs.push({ name, node });
    }
  }
}

// ── Event registration ───────────────────────────────────────────

/**
 * Register an element event, choosing global delegation or direct binding.
 *
 * Bubbling-and-composed events route through the document-level dispatcher;
 * non-bubbling events use a direct listener on the target.
 */
function addEvent(
  host: EventStateHost,
  target: EventTarget,
  eventName: string,
  handlerName: string,
  needsEvent: number,
  owner: TemplateInstance | undefined,
): void {
  if (canDelegateEvent(eventName)) {
    addDelegatedEvent(host, target, eventName, handlerName);
    return;
  }
  addDirectEvent(host, target, eventName, handlerName, needsEvent, owner);
}

/** Register a target with the global delegated event bucket. */
function addDelegatedEvent(
  host: EventStateHost,
  target: EventTarget,
  eventName: string,
  handlerName: string,
): void {
  const method = host[handlerName];
  if (typeof method !== 'function') return;
  const bucket = getGlobalBucket(eventName);
  const entry: HandlerEntry = { host, handler: method as RuntimeEventHandler };
  const existing = bucket.targets.get(target);
  if (existing === undefined) {
    bucket.targets.set(target, entry);
    return;
  }
  if (Array.isArray(existing)) {
    existing.push(entry);
    return;
  }
  bucket.targets.set(target, [existing, entry]);
}

/**
 * Attach a direct event listener for root or non-bubbling events.
 *
 * Uses the binding object itself as the DOM `EventListener` (via
 * `handleEvent`) — avoiding the per-listener closure that `.bind(host)`
 * would allocate.
 */
function addDirectEvent(
  host: EventStateHost,
  target: EventTarget,
  eventName: string,
  handlerName: string,
  _needsEvent: number,
  owner?: TemplateInstance,
): void {
  const method = host[handlerName];
  if (typeof method !== 'function') return;
  const binding = new DirectEventBinding(
    target,
    eventName,
    host,
    method as RuntimeEventHandler,
    owner,
  );
  target.addEventListener(eventName, binding);
  let arr = directBindings.get(host);
  if (!arr) {
    arr = [];
    directBindings.set(host, arr);
  }
  arr.push(binding);
}

// ── Cleanup ──────────────────────────────────────────────────────

/**
 * Mark host as destroyed and remove all direct event listeners.
 *
 * Delegated bindings are NOT explicitly cleaned: the WeakMap that backs
 * each global bucket auto-releases entries when their target element is
 * garbage collected. Adding to `deadHosts` ensures any straggling
 * dispatch through still-live targets becomes a no-op.
 */
export function removeAllEvents(host: EventStateHost): void {
  deadHosts.add(host);
  const direct = directBindings.get(host);
  if (direct) {
    for (let i = 0; i < direct.length; i++) {
      const binding = direct[i];
      binding.target.removeEventListener(binding.eventName, binding);
    }
    directBindings.delete(host);
  }
}

/**
 * Remove direct event listeners owned by the given template instances.
 *
 * Used during `$disposeInstance` to clean up listeners for destroyed
 * blocks without touching listeners owned by other blocks. Delegated
 * bindings inside removed instances clean up automatically when their
 * target elements are GC'd.
 */
export function removeDirectEventsForInstances(host: EventStateHost, instances: TemplateInstance[]): void {
  const direct = directBindings.get(host);
  if (!direct || direct.length === 0) return;

  const singleOwner = instances.length === 1 ? instances[0] : null;
  let owners: Set<TemplateInstance> | null = null;
  if (!singleOwner) {
    owners = new Set<TemplateInstance>();
    for (let i = 0; i < instances.length; i++) owners.add(instances[i]);
  }

  let write = 0;
  for (let i = 0; i < direct.length; i++) {
    const binding = direct[i];
    const shouldRemove = singleOwner
      ? binding.owner === singleOwner
      : !!binding.owner && owners !== null && owners.has(binding.owner);
    if (shouldRemove) {
      binding.target.removeEventListener(binding.eventName, binding);
    } else {
      direct[write] = binding;
      write++;
    }
  }
  direct.length = write;
  if (write === 0) directBindings.delete(host);
}
