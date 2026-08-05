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
 * The interactive element base. Authored components extend this to gain event
 * binding (`@click`, root events), decorator-backed state, `w-ref` wiring, and
 * `$emit`. HTML-only components never reach this class.
 */
export class WebUIElement extends TemplateElement {
  /**
   * Defer SSR hydration until this component is within 200px of the viewport.
   *
   * Server-rendered DOM remains visible and interaction, focus, or keyboard
   * input activates the component synchronously. Client-created instances mount
   * eagerly. Subclasses opt in with `static lazy = true`.
   */
  static lazy = false;

  protected override $shouldDeferSSRHydration(): boolean {
    return (
      (this.constructor as typeof WebUIElement).lazy === true &&
      supportsLazyHydration()
    );
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
      (this.constructor as typeof WebUIElement).lazy === true &&
      supportsLazyHydration()
    ) {
      observeStreamedLazyHydration(this, state);
      return;
    }
    super.$activateDeferredSSR(state);
  }

  [LAZY_HYDRATION_ACTIVATE](
    state: Record<string, unknown> | undefined,
  ): void {
    // Call virtually so an authored override that instruments or extends the
    // protected activation hook retains its existing behavior.
    this.$activateDeferredSSR(state);
  }

  override disconnectedCallback(): void {
    disconnectLazyHydration(this);
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
