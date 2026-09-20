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
  TemplateNodeIndex,
} from './template.js';
import type {
  ScopeFrame,
  TemplateInstance,
} from './element/types.js';

type EventHandler = (...args: unknown[]) => unknown;

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

// ── Development build flag ──────────────────────────────────────
// See the identical `__WEBUI_DEV__` note in `template-element.ts`: a
// bundler-folded compile-time constant, declared and consumed module-locally
// so a production build (`--define:__WEBUI_DEV__=false`) can constant-fold
// `DEV` to `false` and dead-code-eliminate `warnMissingLazyHydrationEntry`'s
// body, while `tsc`, unit tests, and `webui-press serve` keep it active.
declare const __WEBUI_DEV__: boolean;
const DEV: boolean = typeof __WEBUI_DEV__ === 'undefined' || __WEBUI_DEV__;

let warnedMissingLazyHydrationEntry = false;

/**
 * Warn once, in development, when a visibility-deferred component falls back
 * to eager hydration because the optional
 * `@microsoft/webui-framework/lazy-hydration.js` entry was never imported.
 * Never warns for a missing `IntersectionObserver` — that fallback is
 * documented, expected behavior on older browsers, not a misconfiguration.
 */
function warnMissingLazyHydrationEntry(tag: string, policy: 1 | 2): void {
  if (!DEV || warnedMissingLazyHydrationEntry) return;
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
      warnMissingLazyHydrationEntry(this.tagName, policy);
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
    syncAttrProperties(this, this.constructor as Function);
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
    root: Node,
    meta: TemplateBlockMeta,
    resolver: (root: Node, path: TemplateNodeIndex) => Node | null,
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
    resolver: (root: Node, path: TemplateNodeIndex) => Node | null,
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

  /**
   * Wire root-level events from the `<template>` wrapper onto the host element.
   *
   * The host observes events dispatched on the host itself (which never enter
   * the shadow tree) plus every `composed` event on its way out of it.
   * Non-composed events (`change`, `submit`, `select`, media) stop at the
   * shadow root by design and are bound per element instead. `event.target` is
   * retargeted to the host for inner events; use `event.composedPath()[0]`.
   */
  private $wireRoot(instance: TemplateInstance, re: [string, string, CompiledEventArgs][]): void {
    for (let i = 0; i < re.length; i++) {
      this.$addEvent(instance, this, re[i][0], re[i][1], re[i][2], undefined);
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
