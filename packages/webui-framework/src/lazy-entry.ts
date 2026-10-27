// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Lazy-region runtime for `<if condition="..." lazy>`.
 *
 * Loads the build-time template pack of a region when it activates or when its
 * prefetch trigger fires, and releases inactive region templates at idle.
 * Import once before `startRuntime()`. The rendering core carries no lazy
 * code: this module extends the element, so pages without lazy regions pay
 * nothing.
 */

import { installLazyResolver } from './lazy-contract.js';
import {
  activate,
  blockOwner,
  disposed,
  opened,
  prefetchPacks,
  rendered,
  resolve,
  type LazyHost,
} from './lazy-runtime.js';
import { isConditionTruthy } from './template-conditions.js';
import { TemplateElement } from './template-element.js';
import type { CondBinding, TemplateInstance } from './element/types.js';
import type { TemplateMeta } from './template-types.js';

declare module './template-element.js' {
  interface TemplateElement {
    /**
     * Resolve once every active lazy region of this component has rendered.
     *
     * Applies queued updates first, so `this.open = true; await this.$rendered()`
     * waits for the region that the assignment activated, including nested
     * regions that it activates. Rejects when a pack fails to load; calling
     * again, or the next activation, retries it.
     */
    $rendered(): Promise<void>;
    /**
     * Load this component's lazy regions without rendering them.
     *
     * Regions also prefetch on their compiled trigger; call this for custom
     * signals such as a route change or a keyboard shortcut.
     */
    $prefetch(): Promise<void>;
  }
}

/** Element internals this runtime extends. */
interface Host extends LazyHost {
  $meta?: TemplateMeta;
  $conditionResolver(): (path: string, scope?: unknown) => unknown;
  $disposeInstance(root: TemplateInstance, removeNodes: boolean, retainFragments?: boolean): void;
  $flushUpdates(): void;
  $templateMeta(): TemplateMeta | undefined;
  $rendered(): Promise<void>;
  $prefetch(): Promise<void>;
}

const element = TemplateElement.prototype as unknown as Host;
const toggle = element.$toggleCond;
const dispose = element.$disposeInstance;

element.$toggleCond = function (this: Host, c: CondBinding): void {
  const region = c.blockIndex;
  if (typeof region !== 'string' || c.instance) {
    toggle.call(this, c);
    return;
  }
  // A closed region needs its template only once its condition holds.
  if (!c.condition[0](this.$conditionResolver(), c.scope, isConditionTruthy)) return;
  const url = c.anchor?.parentNode && !resolve(region, this.localName, true)
    ? (c.owner.fragment ?? this.$meta)?.lp?.[region]
    : undefined;
  if (url) {
    activate(this, c, url);
    return;
  }
  toggle.call(this, c);
  if (c.instance) opened(region, c);
};

element.$disposeInstance = function (
  this: Host,
  root: TemplateInstance,
  removeNodes: boolean,
  retainFragments?: boolean,
): void {
  dispose.call(this, root, removeNodes, retainFragments);
  disposed();
};

element.$rendered = function (this: Host): Promise<void> {
  this.$flushUpdates();
  return rendered(this);
};

element.$prefetch = function (this: Host): Promise<void> {
  return prefetchPacks((this.$meta ?? this.$templateMeta())?.lp);
};

installLazyResolver(resolve, blockOwner);
