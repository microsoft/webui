// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * WebUIElement — authored interactive Web Component base class.
 *
 * The framework runtime is tiered for Interactive Islands:
 * - `template-element.ts` owns compiled-template hydration, hidden template state,
 *   repeat/conditional updates, and DOM patching.
 * - `WebUIElement` adds authored interactivity: decorators, event handlers,
 *   root events, `w-ref`, and `$emit`.
 *
 * Scriptless components use compiler-owned hosts and never import this authored
 * behavior layer. Templates with DOM or reactive work use TemplateElement;
 * templates with no DOM, reactive, or CSS work use a minimal HTMLElement host.
 */

import { TemplateElement } from './template-element.js';
import { invokeRenderCallback } from './render-completion.js';
import { FLUSH_UPDATES } from './render-scheduler.js';
import {
  disconnectLazyHydration,
  isStreamedLazyActivation,
  isLazyHydrationCoordinatorInstalled,
  LAZY_HYDRATION_ACTIVATE,
  LAZY_HYDRATION_CONTENT_VISIBILITY,
  LAZY_HYDRATION_VIEWPORT,
  observeLazyHydration,
  observeStreamedLazyHydration,
  supportsLazyHydration,
  type LazyHydrationMode,
} from './lazy-hydration-contract.js';
import {
  isStreamingHydrationMode,
  STREAMED_HOST_ATTR,
} from './streaming-mode.js';
import {
  attributeNameForProperty,
  getObservableNames,
  syncAttrProperties,
} from './decorators.js';
import type {
  CompiledEventArg,
  CompiledEventArgs,
  TemplateBlockMeta,
  TemplateMeta,
} from './template.js';
import type {
  ScopeFrame,
  TemplateEventListener,
  TemplateInstance,
} from './element/types.js';

type EventHandler = (...args: unknown[]) => unknown;

interface BoundTemplateEvent extends TemplateEventListener {
  readonly owner: WebUIElement;
  readonly method: EventHandler;
  readonly args: CompiledEventArgs;
  readonly scope: ScopeFrame | undefined;
}

function resolveEventArg(owner: TemplateElement, arg: CompiledEventArg, event: Event, scope?: ScopeFrame): unknown {
  switch (arg[0]) {
    case 'e': return event;
    case 'p': return owner.$resolveValue(arg[1], scope);
    case 's': return arg[1];
    case 'n': return arg[1];
    case 'b': return !!arg[1];
    case 'z': return null;
  }
}

/**
 * Per-instance escape hatches. `w-render="eager"` disables the complete
 * lazy policy; `w-hydrate="eager"` keeps rendering deferral but wires the
 * SSR component synchronously.
 */
const HYDRATE_ATTR = 'w-hydrate';
const RENDER_ATTR = 'w-render';
const HYDRATE_EAGER = 'eager';
const INTERACTION_HYDRATION = 3;
const LAZY_RENDER_INTERACTION = 4;
const AUTHORED_LIVE = 1;
const AUTHORED_PENDING = 2;
const AUTHORED_FLUSHING = 4;

// ── Development build flag ──────────────────────────────────────
// Guard the warning at its call site so production bundlers can remove it.
declare const __WEBUI_DEV__: boolean;

let warnedMissingLazyHydrationEntry = false;

/**
 * Warn once, in development, when a visibility-deferred component falls back
 * to eager hydration because the optional
 * `@microsoft/webui-framework/lazy-hydration.js` entry was never imported.
 * Never warns for a missing `IntersectionObserver` — that fallback is
 * documented, expected behavior on older browsers, not a misconfiguration.
 */
function warnMissingLazyHydrationEntry(tag: string, policy: 1 | 2): void {
  if (warnedMissingLazyHydrationEntry) return;
  warnedMissingLazyHydrationEntry = true;
  const directive = policy === LAZY_HYDRATION_CONTENT_VISIBILITY
    ? 'w-render="lazy"'
    : 'w-hydrate="lazy"';
  console.warn(
    `[WebUI] <${tag}> was compiled with \`${directive}\`, but ` +
    "'@microsoft/webui-framework/lazy-hydration.js' was never imported, " +
    'so it hydrated eagerly instead. Import that optional entry once before ' +
    'your component definitions to enable visibility-deferred hydration.',
  );
}

/**
 * The interactive element base. Authored components extend this to gain event
 * binding (`@click`, root events), decorator-backed state, `w-ref` wiring, and
 * `$emit`. HTML-only components never reach this class.
 */
export class WebUIElement extends TemplateElement {
  private $lazyHydrationMode: LazyHydrationMode | undefined;
  // Only LIVE dispatches directly. Pending/flushing effects retain their
  // original old values, and disconnect clears LIVE without discarding them.
  declare private $authoredPhase: number | undefined;
  declare private $pendingPropertyChanges: Map<string, unknown> | undefined;
  declare private $preUpgradeProperties: Map<string, unknown> | undefined;

  constructor() {
    super();
    const names = getObservableNames(this.constructor as Function);
    for (const name of names) {
      if (!Object.hasOwn(this, name)) continue;
      (this.$preUpgradeProperties ??= new Map()).set(
        name, (this as Record<string, unknown>)[name],
      );
      delete (this as Record<string, unknown>)[name];
    }
  }

  override connectedCallback(): void {
    const properties = this.$preUpgradeProperties;
    if (properties) {
      this.$preUpgradeProperties = undefined;
      for (const [name, value] of properties) {
        (this as Record<string, unknown>)[name] = value;
      }
    }
    super.connectedCallback();
  }

  override $destroy(): void {
    if (this.$authoredPhase !== undefined) this.$authoredPhase &= ~AUTHORED_LIVE;
    super.$destroy();
  }

  $recordPropertyChange(name: string, oldValue: unknown): void {
    if (this.$authoredPhase === undefined) return;
    const changes = this.$pendingPropertyChanges;
    if ((this.$authoredPhase & AUTHORED_LIVE) && this.$canRunAuthoredEffects()) {
      if (changes?.has(name)) {
        oldValue = changes.get(name);
        changes.delete(name);
      }
      try {
        this.$invokePropertyChange(name, oldValue);
      } finally {
        this.$releasePropertyChanges();
      }
    } else {
      const queued = changes ?? (this.$pendingPropertyChanges = new Map());
      if (!queued.has(name)) queued.set(name, oldValue);
      this.$authoredPhase |= AUTHORED_PENDING;
    }
  }

  protected override $reconcileAuthoredState(): boolean {
    if (!this.$canRunAuthoredEffects()) return !this.$pendingPropertyChanges;
    if (this.$authoredPhase === undefined) {
      const names = getObservableNames(this.constructor as Function);
      for (const name of names) {
        if (typeof (this as Record<string, unknown>)[`${name}Changed`] !== 'function') continue;
        this.$authoredPhase = AUTHORED_LIVE;
        if (Object.hasOwn(this, `_${name}`)) {
          (this.$pendingPropertyChanges ??= new Map()).set(name, undefined);
        }
      }
    }
    if (this.$authoredPhase === undefined) return true;
    this.$authoredPhase |= AUTHORED_LIVE;
    if (this.$authoredPhase & AUTHORED_FLUSHING) return false;
    if (this.$pendingPropertyChanges) {
      this.$authoredPhase |= AUTHORED_PENDING;
      this.$flushAuthoredChanges();
    }
    return !this.$pendingPropertyChanges;
  }

  override [FLUSH_UPDATES](): void {
    if (this.$pendingPropertyChanges) this.$notifyHydrated();
    super[FLUSH_UPDATES]();
  }

  private $flushAuthoredChanges(): void {
    const changes = this.$pendingPropertyChanges;
    if (!changes || !this.$canRunAuthoredEffects()) return;
    this.$authoredPhase = AUTHORED_LIVE | AUTHORED_PENDING | AUTHORED_FLUSHING;
    try {
      for (const [name, oldValue] of changes) {
        if (!this.$canRunAuthoredEffects()) {
          this.$authoredPhase &= ~AUTHORED_LIVE;
          break;
        }
        changes.delete(name);
        this.$invokePropertyChange(name, oldValue);
      }
    } finally {
      this.$authoredPhase &= ~AUTHORED_FLUSHING;
      this.$releasePropertyChanges();
    }
  }

  private $invokePropertyChange(name: string, oldValue: unknown): void {
    const value = (this as Record<string, unknown>)[name];
    if (Object.is(oldValue, value)) return;
    const callback = (this as Record<string, unknown>)[`${name}Changed`];
    if (typeof callback === 'function') {
      invokeRenderCallback(this, callback as (old: unknown, next: unknown) => void, oldValue, value);
    }
  }

  private $releasePropertyChanges(): void {
    const phase = this.$authoredPhase;
    if (phase !== undefined && this.$pendingPropertyChanges?.size === 0 &&
        !(phase & AUTHORED_FLUSHING)) {
      this.$pendingPropertyChanges = undefined;
      this.$authoredPhase = phase & ~AUTHORED_PENDING;
    }
  }

  protected override $shouldDeferSSRHydration(meta?: TemplateMeta): boolean {
    this.$lazyHydrationMode = this.$resolveLazyHydrationMode(meta);
    return this.$lazyHydrationMode !== undefined;
  }

  protected override $didDeferSSRHydration(): void {
    // A streamed host cannot hydrate until its boundary-local state arrives.
    if (
      !isStreamingHydrationMode() ||
      !this.hasAttribute(STREAMED_HOST_ATTR)
    ) {
      this.$primeSSRStateForDeferral();
      const mode = this.$lazyHydrationMode;
      if (mode !== undefined) observeLazyHydration(this, mode);
    }
  }

  protected override $didDeferStreamedSSRHydration(
    state: Record<string, unknown> | undefined,
  ): void {
    const mode = this.$lazyHydrationMode;
    if (mode === undefined) {
      super.$didDeferStreamedSSRHydration(state);
      return;
    }
    observeStreamedLazyHydration(this, state, mode);
  }

  protected override $activateDeferredSSR(
    state?: Record<string, unknown>,
  ): void {
    if (isStreamedLazyActivation(this)) {
      this.$lazyHydrationMode = undefined;
      super.$activateDeferredSSRFromBoundary(state);
      return;
    }
    const mode = this.$lazyHydrationMode ??
      this.$resolveLazyHydrationMode(this.$currentTemplateMetadata());
    if (
      isStreamingHydrationMode() &&
      this.hasAttribute(STREAMED_HOST_ATTR) &&
      mode !== undefined
    ) {
      this.$lazyHydrationMode = mode;
      observeStreamedLazyHydration(this, state, mode);
      return;
    }
    this.$lazyHydrationMode = undefined;
    super.$activateDeferredSSR(state);
  }

  /**
   * Whether this SSR instance should defer to the viewport/interaction
   * coordinator. Checks compact compiler metadata first, so an ordinary eager
   * component returns without reading an attribute or touching the optional
   * coordinator contract.
   */
  private $resolveLazyHydrationMode(
    meta?: TemplateMeta,
  ): LazyHydrationMode | undefined {
    const policy = meta?.wp;
    if (policy === undefined) return undefined;
    if (
      policy === INTERACTION_HYDRATION
      || policy === LAZY_RENDER_INTERACTION
    ) return undefined;
    if (
      policy !== LAZY_HYDRATION_VIEWPORT
      && policy !== LAZY_HYDRATION_CONTENT_VISIBILITY
    ) return undefined;
    if (
      policy === LAZY_HYDRATION_CONTENT_VISIBILITY &&
      this.getAttribute(RENDER_ATTR) === HYDRATE_EAGER
    ) return undefined;
    if (this.getAttribute(HYDRATE_ATTR) === HYDRATE_EAGER) return undefined;
    if (!isLazyHydrationCoordinatorInstalled()) {
      if (typeof __WEBUI_DEV__ === 'undefined' || __WEBUI_DEV__) {
        warnMissingLazyHydrationEntry(this.tagName, policy);
      }
      return undefined;
    }
    if (!supportsLazyHydration()) return undefined;
    return policy === LAZY_HYDRATION_CONTENT_VISIBILITY
      ? LAZY_HYDRATION_CONTENT_VISIBILITY
      : LAZY_HYDRATION_VIEWPORT;
  }

  [LAZY_HYDRATION_ACTIVATE](
    state: Record<string, unknown> | undefined,
  ): void {
    // Call virtually so an authored override that instruments or extends the
    // protected activation hook retains its existing behavior.
    this.$activateDeferredSSR(state);
  }

  override disconnectedCallback(): void {
    if (this.$authoredPhase !== undefined) this.$authoredPhase &= ~AUTHORED_LIVE;
    if (this.$lazyHydrationMode !== undefined) {
      disconnectLazyHydration(this);
    }
    super.disconnectedCallback();
  }

  protected override $observableNames(): Set<string> {
    return getObservableNames(this.constructor as Function);
  }

  protected override $shouldApplySSRState(key: string): boolean {
    const attribute = attributeNameForProperty(
      this.constructor as Function,
      key,
    );
    return attribute === undefined || !this.hasAttribute(attribute);
  }

  protected override $syncAuthoredAttributes(): void {
    if (this.isConnected) syncAttrProperties(this, this.constructor as Function);
  }

  /** Dispatch a bubbling, composed custom event for parent communication. */
  $emit(name: string, detail?: unknown): boolean {
    return this.dispatchEvent(
      new CustomEvent(name, {
        bubbles: true,
        cancelable: true,
        composed: true,
        detail,
      }),
    );
  }

  /** Wire events + root events + refs (shared by $wire and $hydrate). */
  protected override $finalize(
    instance: TemplateInstance,
    meta: TemplateBlockMeta,
    elements: readonly (Node | undefined)[],
  ): void {
    if (meta.eg || (meta as TemplateMeta).re) {
      this.$wireEvents(instance, meta, elements);
    }
    if (meta.rf) wireRefs(this as Record<string, unknown>, instance, meta, elements);
  }

  private $wireEvents(
    instance: TemplateInstance,
    meta: TemplateBlockMeta,
    elements: readonly (Node | undefined)[],
  ): void {
    const groups = meta.eg;
    const rootEvents = (meta as TemplateMeta).re;
    let capacity = rootEvents?.length ?? 0;
    if (groups) {
      for (let i = 0; i < groups.length; i++) capacity += groups[i][1].length;
    }
    if (capacity > 0) {
      const listeners = new Array<TemplateEventListener>(capacity);
      let count = 0;
      if (groups) {
        for (let i = 0; i < groups.length; i++) {
          const [eventName, bindings] = groups[i];
          for (let j = 0; j < bindings.length; j++) {
            const [handlerName, args, target] = bindings[j];
            const el = elements[target];
            if (el && el.nodeType === 1) {
              const listener = this.$addEvent(el, eventName, handlerName, args, instance.scope);
              if (listener) listeners[count++] = listener;
            }
          }
        }
      }
      // Only composed inner events reach these host listeners.
      if (rootEvents) {
        for (let i = 0; i < rootEvents.length; i++) {
          const listener = this.$addEvent(this, rootEvents[i][0], rootEvents[i][1], rootEvents[i][2]);
          if (listener) listeners[count++] = listener;
        }
      }
      listeners.length = count;
      instance.listeners = listeners;
    }
  }

  /** Attach a direct listener for an event binding. */
  private $addEvent(
    target: EventTarget,
    eventName: string,
    handlerName: string,
    args: CompiledEventArgs,
    scope?: ScopeFrame,
  ): BoundTemplateEvent | undefined {
    const method = (this as Record<string, unknown>)[handlerName];
    if (typeof method !== 'function') return;
    const listener: BoundTemplateEvent = {
      owner: this, target, eventName, method: method as EventHandler, args, scope,
      handleEvent: handleTemplateEvent,
    };
    target.addEventListener(eventName, listener);
    return listener;
  }
}

function wireRefs(
  owner: Record<string, unknown>,
  instance: TemplateInstance,
  meta: TemplateBlockMeta,
  elements: readonly (Node | undefined)[],
): void {
  const refs = meta.rf;
  if (!refs) return;
  for (let i = 0; i < refs.length; i++) {
    const [name, target] = refs[i];
    const node = elements[target];
    if (!node || node.nodeType !== 1) continue;
    owner[name] = node;
    // Component roots retain their DOM; only removable views release refs.
    if (!(meta as TemplateMeta).v || meta.f) {
      (instance.cleanups ??= []).push(removeNodes => {
        if (removeNodes && owner[name] === node) owner[name] = undefined;
      });
    }
  }
}

function handleTemplateEvent(this: BoundTemplateEvent, event: Event): void {
  const { owner, method, args, scope } = this;
  switch (args.length) {
    case 0:
      method.call(owner);
      return;
    case 1:
      method.call(owner, resolveEventArg(owner, args[0], event, scope));
      return;
    case 2:
      method.call(
        owner,
        resolveEventArg(owner, args[0], event, scope),
        resolveEventArg(owner, args[1], event, scope),
      );
      return;
    case 3:
      method.call(
        owner,
        resolveEventArg(owner, args[0], event, scope),
        resolveEventArg(owner, args[1], event, scope),
        resolveEventArg(owner, args[2], event, scope),
      );
      return;
    case 4:
      method.call(
        owner,
        resolveEventArg(owner, args[0], event, scope),
        resolveEventArg(owner, args[1], event, scope),
        resolveEventArg(owner, args[2], event, scope),
        resolveEventArg(owner, args[3], event, scope),
      );
      return;
    default: {
      const values = new Array<unknown>(args.length);
      for (let i = 0; i < args.length; i++) {
        values[i] = resolveEventArg(owner, args[i], event, scope);
      }
      method.apply(owner, values);
    }
  }
}
