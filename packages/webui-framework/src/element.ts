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
 * HTML-only static hosts import the template tier directly, so scriptless
 * components no longer pull decorator/event/ref code into their bundles.
 */

import { TemplateElement } from './template-element.js';
import {
  getObservableNames,
  isAttributeProperty,
  syncAttrProperties,
} from './decorators.js';
import type {
  CompiledEventArg,
  CompiledEventArgs,
  CompiledEventBindingMeta,
  TemplateBlockMeta,
  TemplateMeta,
  TemplateNodePath,
} from './template.js';
import type {
  ScopeFrame,
  TemplateInstance,
} from './element/types.js';

type EventHandler = (...args: unknown[]) => unknown;

type DelegatedEventEntry = {
  target: Element;
  method: EventHandler;
  args: CompiledEventArgs;
  usesEvent: boolean;
  scope?: ScopeFrame;
};

/**
 * The interactive element base. Authored components extend this to gain event
 * binding (`@click`, root events), decorator-backed state, `w-ref` wiring, and
 * `$emit`. HTML-only components never reach this class.
 */
export class WebUIElement extends TemplateElement {
  protected override $observableNames(): Set<string> {
    return getObservableNames(this.constructor as Function);
  }

  protected override $shouldApplySSRState(key: string): boolean {
    return !isAttributeProperty(this.constructor as Function, key);
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

  /** Wire element events as one delegated listener per event name. */
  private $wireEvents(
    instance: TemplateInstance,
    root: Node,
    meta: TemplateBlockMeta,
    resolver: (root: Node, path: TemplateNodePath) => Node | null,
    scope?: ScopeFrame,
  ): void {
    const groups = meta.eg;
    if (!groups) return;
    const delegateTarget = this.shadowRoot ?? this;
    for (let i = 0; i < groups.length; i++) {
      const [eventName, bindings] = groups[i];
      const bucket = this.$resolveDelegatedEvents(root, bindings, resolver, scope);
      this.$addDelegatedEvent(instance, delegateTarget, eventName, bucket);
    }
  }

  private $resolveDelegatedEvents(
    root: Node,
    bindings: CompiledEventBindingMeta[],
    resolver: (root: Node, path: TemplateNodePath) => Node | null,
    scope?: ScopeFrame,
  ): DelegatedEventEntry[] {
    const entries: DelegatedEventEntry[] = [];
    for (let i = 0; i < bindings.length; i++) {
      const [handlerName, args, target, usesEvent] = bindings[i];
      const el = resolver(root, target);
      if (!el || el.nodeType !== 1) continue;
      const method = (this as Record<string, unknown>)[handlerName];
      if (typeof method !== 'function') continue;
      entries.push({
        target: el as Element,
        method: method as EventHandler,
        args,
        usesEvent: usesEvent === 1,
        scope,
      });
    }
    return entries;
  }

  /** Wire root-level events on the host element (or shadow root when present). */
  private $wireRoot(instance: TemplateInstance, re: [string, string, CompiledEventArgs][]): void {
    const target = this.shadowRoot ?? this;
    for (let i = 0; i < re.length; i++) {
      this.$addEvent(instance, target, re[i][0], re[i][1], re[i][2], undefined);
    }
  }

  /** Attach one listener for all bindings of the same event name in an instance. */
  private $addDelegatedEvent(
    instance: TemplateInstance,
    target: EventTarget,
    eventName: string,
    entries: DelegatedEventEntry[],
  ): void {
    if (entries.length === 0) return;
    const listener = (event: Event): void => {
      this.$dispatchDelegatedEvent(entries, event);
    };
    target.addEventListener(eventName, listener);
    this.$addCleanup(instance, () => target.removeEventListener(eventName, listener));
  }

  private $dispatchDelegatedEvent(entries: DelegatedEventEntry[], event: Event): void {
    let current = event.target as Node | null;
    while (current) {
      for (let i = 0; i < entries.length; i++) {
        const entry = entries[i];
        if (entry.target === current) {
          this.$callEventHandler(entry.method, entry.args, event, entry.scope, entry.target, entry.usesEvent);
        }
      }
      current = current.parentNode;
    }
  }

  /** Attach a direct listener for root-level event bindings. */
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
    currentTarget?: EventTarget,
    usesEvent = false,
  ): void {
    if (currentTarget && usesEvent) {
      this.$callEventHandlerWithCurrentTarget(method, args, event, scope, currentTarget);
      return;
    }
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

  private $callEventHandlerWithCurrentTarget(
    method: EventHandler,
    args: CompiledEventArgs,
    event: Event,
    scope: ScopeFrame | undefined,
    currentTarget: EventTarget,
  ): void {
    const descriptor = Object.getOwnPropertyDescriptor(event, 'currentTarget');
    Object.defineProperty(event, 'currentTarget', {
      configurable: true,
      value: currentTarget,
    });
    try {
      this.$callEventHandler(method, args, event, scope);
    } finally {
      if (descriptor) {
        Object.defineProperty(event, 'currentTarget', descriptor);
      } else {
        delete (event as { currentTarget?: EventTarget | null }).currentTarget;
      }
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
