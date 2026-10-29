// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Lazy-region runtime for `<if condition="..." lazy>`.
 *
 * Loads the build-time template pack and the custom element scripts of a
 * region when it activates or when its prefetch trigger fires, and releases
 * inactive region templates at idle. Import once before `startRuntime()`,
 * directly or through the generated `lazy.webui.js`. The rendering core
 * carries no lazy code: this module extends the element, so pages without
 * lazy regions pay nothing.
 */

import { installLazyResolver } from './lazy-contract.js';
import {
  activate,
  afterLoad,
  blockOwner,
  disposed,
  hydrated,
  opened,
  prefetchRegions,
  rendered,
  resolve,
  unloaded,
  type LazyHost,
} from './lazy-runtime.js';
import { addScripts, warnStartupDefinitions, type LazyScripts } from './lazy-scripts.js';
import { isConditionTruthy } from './template-conditions.js';
import { TemplateElement } from './template-element.js';
import { hasPendingTemplateDefinition } from './template-registry.js';
import type { CondBinding, TemplateInstance } from './element/types.js';
import type { TemplateMeta } from './template-types.js';

export type { LazyScriptLoader, LazyScripts } from './lazy-scripts.js';

// `__WEBUI_DEV__` is a compile-time constant (see `template-element.ts`). The
// check below uses the define expression itself: bundlers do not inline a
// module-local alias in a module with imports, which would keep the
// development-only guard in production builds.
declare const __WEBUI_DEV__: boolean;

declare module './template-element.js' {
  interface TemplateElement {
    /**
     * Resolve once every active lazy region of this component has rendered.
     *
     * Setting a lazy condition is enough to show its body; await this only when
     * code needs the body's DOM right away, such as before `showModal()`. It
     * applies queued updates first, so `this.open = true; await this.$rendered()`
     * waits for the region that the assignment activated, including nested
     * regions that it activates, and for the custom elements they introduce.
     * Rejects when a pack or script fails to load; calling again, or the next
     * activation, retries it.
     */
    $rendered(): Promise<void>;
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
  $conditionResolver(): (path: string, scope?: unknown) => unknown;
  $disposeInstance(root: TemplateInstance, removeNodes: boolean, retainFragments?: boolean): void;
  $flushUpdates(): void;
  $mount(...args: unknown[]): void;
  $prefetch(path?: string): void;
  $rendered(): Promise<void>;
}

const element = TemplateElement.prototype as unknown as Host;
const toggle = element.$toggleCond;
const dispose = element.$disposeInstance;
const mount = element.$mount;
/** The host whose template is hydrating or mounting. */
let mounting: Host | undefined;

element.$toggleCond = function (this: Host, c: CondBinding): void {
  const region = c.blockIndex;
  if (typeof region !== 'string' || c.instance) {
    toggle.call(this, c);
    return;
  }
  // A closed region needs its template only once its condition holds.
  if (!c.condition[0](this.$conditionResolver(), c.scope, isConditionTruthy)) return;
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
  mounting = this;
  try {
    mount.apply(this, args);
  } finally {
    mounting = previous;
  }
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

element.$prefetch = function (this: Host, path?: string): void {
  prefetchRegions(this.$meta, path);
};

installLazyResolver(resolve, meta => {
  // A region body rendered by the server hydrates with its templates but not
  // its scripts, so they load now at activation priority.
  const region = (meta as TemplateMeta).f;
  if (region && mounting) hydrated(mounting, region);
  return blockOwner(meta);
});

/**
 * Register the loaders of the custom elements that lazy regions introduce.
 *
 * `webui build` generates `lazy.webui.js`, which calls this with one dynamic
 * import per scripted component; import that module instead of `lazy.js`. A
 * region's scripts load with its pack, and its body mounts once both are ready.
 */
export function defineLazyScripts(scripts: LazyScripts): void {
  addScripts(scripts);
  if (typeof __WEBUI_DEV__ === 'undefined' || __WEBUI_DEV__) {
    // WebUI components wait for their templates to define themselves, so the
    // guard also counts declarations that are still pending.
    afterLoad(() => warnStartupDefinitions(hasPendingTemplateDefinition));
  }
}
