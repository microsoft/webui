// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Lazy-region runtime for `<if condition="..." lazy>`.
 *
 * Loads the build-time template pack and the custom element scripts of a
 * region when it activates or when its prefetch trigger fires, and releases
 * inactive region templates at idle. Import the generated `lazy.webui.js`
 * once before `startRuntime()`. The rendering core
 * carries no lazy code: this module extends the element, so pages without
 * lazy regions pay nothing.
 */

import { installLazyResolver } from './lazy-contract.js';
import {
  activate,
  blockOwner,
  cancelActivation,
  disposeActivations,
  disposed,
  hydrated,
  joinHydratedMount,
  opened,
  prefetchRegions,
  resolve,
  retryFailed,
  unloaded,
  watch,
  type LazyHost,
} from './lazy-runtime.js';
import { installRenderTaskRetry, renderWork } from './render-completion.js';
import { isConditionTruthy } from './template-conditions.js';
import { TemplateElement } from './template-element.js';
import type { CondBinding, TemplateInstance } from './element/types.js';
import type { TemplateMeta } from './template-types.js';

declare module './template-element.js' {
  interface TemplateElement {
    /**
     * Start loading this component's lazy regions before they activate.
     *
     * Without `path`, prefetches every region whose condition is in this
     * component's template, including `demand` regions. With `path`, only
     * regions whose condition reads that state path or a path below it, so
     * `$prefetch('settings')` covers `settings.open`. Call it from the control
     * that opens a `lazy="demand"` region, for example
     * `@pointerenter="{$prefetch('settingsOpen')}"`. Failures are silent; the
     * activation that follows retries and reports them.
     */
    $prefetch(path?: string): void;
  }
}

declare global {
  interface HTMLElementEventMap {
    /**
     * A lazy region of this element failed to load its pack or scripts.
     * Cancel the event to handle the error instead of reporting it.
     */
    lazyerror: CustomEvent<{ error: unknown }>;
  }
}

/** Element internals this runtime extends. */
interface Host extends LazyHost {
  $meta?: TemplateMeta;
  $changeStructure(removedFrom?: TemplateInstance): void;
  $conditionResolver(): (path: string, scope?: unknown) => unknown;
  $disposeInstance(root: TemplateInstance, removeNodes: boolean, retainFragments?: boolean): void;
  $mount(...args: unknown[]): void;
  $prefetch(path?: string): void;
}

const element = TemplateElement.prototype as unknown as Host;
const toggle = element.$toggleCond;
const dispose = element.$disposeInstance;
const mount = element.$mount;
const change = element.$changeStructure;
/** The host whose template is hydrating or mounting. */
let mounting: Host | undefined;

element.$toggleCond = function (this: Host, c: CondBinding): void {
  const region = c.blockIndex;
  if (typeof region !== 'string' || c.instance) {
    toggle.call(this, c);
    return;
  }
  // A closed region needs its template only once its condition holds.
  if (!c.condition[0](this.$conditionResolver(), c.scope, isConditionTruthy)) {
    cancelActivation(this, c);
    return;
  }
  const container = c.anchor?.parentNode ? c.owner.fragment ?? this.$meta : undefined;
  const url = container?.lp?.[region];
  if (url && unloaded(region, url, this.localName)) {
    activate(this, c, url, container!.lc?.[region]);
    return;
  }
  toggle.call(this, c);
  if (c.instance) opened(region, c);
};

element.$mount = function (this: Host, ...args: unknown[]): void {
  const previous = mounting;
  const associated = joinHydratedMount(this);
  const previousRender = associated ? renderWork?.enter(this) : undefined;
  mounting = this;
  try {
    mount.apply(this, args);
  } finally {
    mounting = previous;
    if (associated) renderWork?.leave(this, previousRender);
  }
  watch(this);
};

// Conditionals and repeats report each batch of blocks they render after the
// host mounted; a block may arm intent regions.
element.$changeStructure = function (this: Host, removedFrom?: TemplateInstance): void {
  change.call(this, removedFrom);
  watch(this);
};

element.$disposeInstance = function (
  this: Host,
  root: TemplateInstance,
  removeNodes: boolean,
  retainFragments?: boolean,
): void {
  disposeActivations(this, root);
  dispose.call(this, root, removeNodes, retainFragments);
  disposed();
};

element.$prefetch = function (this: Host, path?: string): void {
  prefetchRegions(this.$meta, path);
};

installRenderTaskRetry(retryFailed);
installLazyResolver(resolve, (meta, instance) => {
  // A region body rendered by the server hydrates with its templates but not
  // its scripts, so they load now at activation priority.
  const region = (meta as TemplateMeta).f;
  if (region && mounting) hydrated(mounting, region, instance);
  return blockOwner(meta);
});

/**
 * @internal Compiler-to-runtime bridge for generated `lazy.webui.js` only.
 * Registration starts no loads. This is not an application loader API.
 */
export { addScripts as registerGeneratedLazyScripts } from './lazy-scripts.js';
