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
 * Scriptless components use the compiler-owned TemplateElement host and never
 * import this authored behavior layer.
 */

import { TemplateElement } from './template-element.js';
import {
  disconnectLazyHydration,
  isStreamedLazyActivation,
  isVisibleHydrationCoordinatorInstalled,
  LAZY_HYDRATION_ACTIVATE,
  observeLazyHydration,
  observeStreamedLazyHydration,
  supportsLazyHydration,
} from './lazy-hydration.js';
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
  TemplateNodePath,
} from './template.js';
import type {
  ScopeFrame,
  TemplateInstance,
} from './element/types.js';

type EventHandler = (...args: unknown[]) => unknown;

/**
 * Component-level hydration strategy. `'eager'` (the universal default)
 * hydrates synchronously like every other WebUI component; `'visible'` defers
 * SSR instances until the shared viewport/interaction coordinator activates
 * them (see `static hydration` below).
 */
export type HydrationStrategy = 'eager' | 'visible';

/**
 * Per-instance SSR escape hatch for a `hydration = 'visible'` class: the
 * exact attribute value `"eager"` suppresses viewport deferral for that one
 * instance. Any other value is ignored — this is a narrow, exact-match
 * contract rather than a general directive, so it costs nothing beyond one
 * attribute read, and only for components that already opted into
 * `'visible'`.
 */
const HYDRATE_ATTR = 'w-hydrate';
const HYDRATE_EAGER = 'eager';

// ── Development build flag ──────────────────────────────────────
// See the identical `__WEBUI_DEV__` note in `template-element.ts`: a
// bundler-folded compile-time constant, declared and consumed module-locally
// so a production build (`--define:__WEBUI_DEV__=false`) can constant-fold
// `DEV` to `false` and dead-code-eliminate `warnMissingVisibleHydrationEntry`'s
// body, while `tsc`, unit tests, and `webui-press serve` keep it active.
declare const __WEBUI_DEV__: boolean;
const DEV: boolean = typeof __WEBUI_DEV__ === 'undefined' || __WEBUI_DEV__;

let warnedMissingVisibleHydrationEntry = false;

/**
 * Warn once, in development, when a `hydration = 'visible'` component falls
 * back to eager hydration because the optional
 * `@microsoft/webui-framework/visible-hydration.js` entry was never imported.
 * Never warns for a missing `IntersectionObserver` — that fallback is
 * documented, expected behavior on older browsers, not a misconfiguration.
 */
function warnMissingVisibleHydrationEntry(tag: string): void {
  if (!DEV || warnedMissingVisibleHydrationEntry) return;
  warnedMissingVisibleHydrationEntry = true;
  console.warn(
    `[WebUI] <${tag}> sets \`static hydration = 'visible'\`, but ` +
    "'@microsoft/webui-framework/visible-hydration.js' was never imported, " +
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
  /**
   * Component-level hydration strategy. Eager is the universal default.
   *
   * Set `static override readonly hydration = 'visible'` to defer SSR instances until
   * they are within 200px of the viewport. Server-rendered DOM remains
   * visible and interaction, focus, or keyboard input activates the
   * component synchronously regardless. Client-created instances always
   * mount eagerly.
   *
   * `'visible'` requires the optional
   * `@microsoft/webui-framework/visible-hydration.js` entry to be imported
   * before component definitions; without it (or without
   * `IntersectionObserver`), the component falls back to eager hydration
   * rather than staying inert. An individual SSR instance can force eager
   * hydration regardless of this static mode with `w-hydrate="eager"`.
   *
   * The policy is readonly because changing it after instances connect would
   * make one class follow inconsistent hydration semantics.
   */
  static readonly hydration: HydrationStrategy = 'eager';

  protected override $shouldDeferSSRHydration(): boolean {
    return this.$isVisibleHydrationEligible();
  }

  protected override $didDeferSSRHydration(): void {
    // A streamed host cannot hydrate until its boundary-local state arrives.
    if (
      !isStreamingHydrationMode() ||
      !this.hasAttribute(STREAMED_HOST_ATTR)
    ) {
      this.$primeSSRStateForDeferral();
      observeLazyHydration(this);
    }
  }

  protected override $activateDeferredSSR(
    state?: Record<string, unknown>,
  ): void {
    if (isStreamedLazyActivation(this)) {
      super.$activateDeferredSSRFromBoundary(state);
      return;
    }
    if (
      isStreamingHydrationMode() &&
      this.hasAttribute(STREAMED_HOST_ATTR) &&
      this.$isVisibleHydrationEligible()
    ) {
      observeStreamedLazyHydration(this, state);
      return;
    }
    super.$activateDeferredSSR(state);
  }

  /**
   * Whether this SSR instance should defer to the viewport/interaction
   * coordinator. Checks the static `hydration` mode first — an ordinary
   * eager component (the common case) returns `false` immediately and never
   * reaches the `w-hydrate` attribute lookup or the coordinator.
   */
  private $isVisibleHydrationEligible(): boolean {
    if ((this.constructor as typeof WebUIElement).hydration !== 'visible') {
      return false;
    }
    if (this.getAttribute(HYDRATE_ATTR) === HYDRATE_EAGER) return false;
    if (!isVisibleHydrationCoordinatorInstalled()) {
      warnMissingVisibleHydrationEntry(this.tagName);
      return false;
    }
    return supportsLazyHydration();
  }

  [LAZY_HYDRATION_ACTIVATE](
    state: Record<string, unknown> | undefined,
  ): void {
    // Call virtually so an authored override that instruments or extends the
    // protected activation hook retains its existing behavior.
    this.$activateDeferredSSR(state);
  }

  override disconnectedCallback(): void {
    if ((this.constructor as typeof WebUIElement).hydration === 'visible') {
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
    syncAttrProperties(this, this.constructor as Function);
  }

  /** Dispatch a bubbling custom event. Uses composed:true when in shadow DOM. */
  $emit(name: string, detail?: unknown): boolean {
    return this.dispatchEvent(
      new CustomEvent(name, {
        bubbles: true,
        cancelable: true,
        composed: !!this.shadowRoot,
        detail,
      }),
    );
  }

  /** Wire events + root events + refs (shared by $wire and $hydrate). */
  protected override $finalize(
    instance: TemplateInstance,
    root: Node,
    meta: TemplateBlockMeta,
    resolver: (root: Node, path: TemplateNodePath) => Node | null,
    scope?: ScopeFrame,
  ): void {
    this.$wireEvents(instance, root, meta, resolver, scope);
    if ((meta as TemplateMeta).re) this.$wireRoot(instance, (meta as TemplateMeta).re!);
    this.$wireRefs(root);
  }

  /**
   * Wire element events.
   *
   * Listeners attach to the bound element, never the render root.
   * `$wireEvents` runs once per block instance, so delegating would stack one
   * listener per block on the same node and fire all of them per dispatch — and
   * would never see non-bubbling events such as `focus`.
   */
  private $wireEvents(
    instance: TemplateInstance,
    root: Node,
    meta: TemplateBlockMeta,
    resolver: (root: Node, path: TemplateNodePath) => Node | null,
    scope?: ScopeFrame,
  ): void {
    const groups = meta.eg;
    if (!groups) return;
    for (let i = 0; i < groups.length; i++) {
      const [eventName, bindings] = groups[i];
      for (let j = 0; j < bindings.length; j++) {
        const [handlerName, args, target] = bindings[j];
        const el = resolver(root, target);
        if (el && el.nodeType === 1) {
          this.$addEvent(instance, el, eventName, handlerName, args, scope);
        }
      }
    }
  }

  /** Wire root-level events on the shadow root when present, otherwise the host element. */
  private $wireRoot(instance: TemplateInstance, re: [string, string, CompiledEventArgs][]): void {
    const target = this.shadowRoot ?? this;
    for (let i = 0; i < re.length; i++) {
      this.$addEvent(instance, target, re[i][0], re[i][1], re[i][2], undefined);
    }
  }

  /** Attach a direct listener for an event binding. */
  private $addEvent(
    instance: TemplateInstance,
    target: EventTarget,
    eventName: string,
    handlerName: string,
    args: CompiledEventArgs,
    scope?: ScopeFrame,
  ): void {
    const method = (this as Record<string, unknown>)[handlerName];
    if (typeof method !== 'function') return;
    const listener = (event: Event): void => this.$callEventHandler(method as EventHandler, args, event, scope);
    target.addEventListener(eventName, listener);
    this.$addCleanup(instance, () => target.removeEventListener(eventName, listener));
  }

  private $addCleanup(instance: TemplateInstance, cleanup: () => void): void {
    (instance.cleanups ??= []).push(cleanup);
  }

  private $callEventHandler(
    method: EventHandler,
    args: CompiledEventArgs,
    event: Event,
    scope?: ScopeFrame,
  ): void {
    switch (args.length) {
      case 0:
        method.call(this);
        return;
      case 1:
        method.call(this, this.$resolveEventArg(args[0], event, scope));
        return;
      case 2:
        method.call(
          this,
          this.$resolveEventArg(args[0], event, scope),
          this.$resolveEventArg(args[1], event, scope),
        );
        return;
      case 3:
        method.call(
          this,
          this.$resolveEventArg(args[0], event, scope),
          this.$resolveEventArg(args[1], event, scope),
          this.$resolveEventArg(args[2], event, scope),
        );
        return;
      case 4:
        method.call(
          this,
          this.$resolveEventArg(args[0], event, scope),
          this.$resolveEventArg(args[1], event, scope),
          this.$resolveEventArg(args[2], event, scope),
          this.$resolveEventArg(args[3], event, scope),
        );
        return;
      default:
        method.apply(this, this.$resolveEventArgs(args, event, scope));
    }
  }

  private $resolveEventArgs(args: CompiledEventArgs, event: Event, scope?: ScopeFrame): unknown[] {
    const resolved: unknown[] = [];
    for (let i = 0; i < args.length; i++) {
      resolved.push(this.$resolveEventArg(args[i], event, scope));
    }
    return resolved;
  }

  private $resolveEventArg(arg: CompiledEventArg, event: Event, scope?: ScopeFrame): unknown {
    switch (arg[0]) {
      case 'e': return event;
      case 'p': return this.$resolveValue(arg[1], scope);
      case 's': return arg[1];
      case 'n': return arg[1];
      case 'b': return !!arg[1];
      case 'z': return null;
    }
  }

  /** Find w-ref attributes and assign to component properties. */
  private $wireRefs(root: Node): void {
    if (root.nodeType !== 1 && root.nodeType !== 11) return;
    const refs = (root as Element).querySelectorAll('[w-ref]');
    for (let i = 0; i < refs.length; i++) {
      const raw = refs[i].getAttribute('w-ref');
      if (!raw || raw.charCodeAt(0) !== 123) continue;
      const name = raw.slice(1, -1);
      if (name) (this as Record<string, unknown>)[name] = refs[i];
    }
  }
}
