// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Pack and script loading, prefetch triggers, activation, and release for lazy regions.
 *
 * `lazy-entry.ts` connects this module to the element. It has no element
 * dependency of its own, so it can be tested without a DOM.
 */

import {
  getTemplate,
  getTemplateRegistry,
  hasPendingTemplateDefinition,
  installTemplateRegistry,
} from './template-registry.js';
import { TEMPLATES_REGISTERED_EVENT } from './template-events.js';
import { runCompilerScript } from './trusted-types-policy.js';
import { resolveAsset } from './asset-base.js';
import { loadScripts, warnUnscriptedComponents } from './lazy-scripts.js';
import { speculativeTemplates } from './speculation.js';
import type { CondBinding } from './element/types.js';
import type { CompiledConditionalMeta, TemplateBlockMeta, TemplateMeta } from './template-types.js';

// A compile-time constant (see `lazy-entry.ts`): checks use the define
// expression itself so production builds drop development-only code.
declare const __WEBUI_DEV__: boolean;

/** The element surface the runtime re-enters once a region's pack loads. */
export interface LazyHost extends Element {
  $toggleCond(binding: CondBinding): void;
}

type Priority = 'high' | 'low' | 'auto';

interface TemplatePack {
  type?: unknown;
  version?: unknown;
  fragment?: unknown;
  required?: readonly string[];
  inventory?: string;
  styleInventory?: string;
  bits?: Readonly<Record<string, number>>;
  componentStyles?: unknown;
  templates?: Record<string, TemplateMeta>;
  templateFunctions?: Readonly<Record<string, string>>;
}

/** A region template this runtime registered and may release. */
interface Registration {
  url: string;
  bit: number | undefined;
  epoch: number;
}

/**
 * A region with a prefetch trigger: identity, pack URL, trigger code, and in
 * link builds the stylesheet paths its pack registers.
 */
type Armed = readonly [region: string, url: string, trigger: number, styles?: readonly string[]];

const INTENT = 1;
const EAGER = 3;
const DEMAND = 4;
const INTENT_EVENTS = ['pointerover', 'focusin', 'touchstart', 'pointerdown'];
/** Matches the delivery request deadline. */
const PACK_TIMEOUT_MS = 10_000;
const noop = (): void => {};
const owners = new WeakMap<TemplateBlockMeta, TemplateMeta>();
const scanned = new WeakSet<TemplateMeta>();
/** Regions with a prefetch trigger, by the block that contains their condition. */
const armable = new WeakMap<TemplateBlockMeta, Armed[]>();
const loads = new Map<string, Promise<void>>();
/** IDs of the templates each pack published before its install last failed; absent once it succeeds. */
const unready = new Map<string, ReadonlySet<string>>();
const registered = new Map<string, Registration>();
const released = new Map<string, Registration & { template: WeakRef<TemplateMeta> }>();
/** Bindings that mounted each region this runtime registered. */
const views = new Map<string, Set<WeakRef<CondBinding>>>();
/**
 * Work each host's active regions wait for: a binding's activation, or the
 * scripts of a region that hydrated open. The value is the in-flight load,
 * or `undefined` after it failed until `$rendered()` retries it.
 */
const pending = new WeakMap<Element, Map<CondBinding | string, Promise<void> | undefined>>();
/** Intent regions waiting for interaction, by owner tag and pack URL. */
let intent: Map<string, Map<string, Armed>> | undefined;
/** Idle and eager prefetch queues, by trigger and pack URL. */
const queues = new Map<number, Map<string, Armed>>();
let releasing = false;

/** A loaded region or shared block, validated against its owner when given. */
export function resolve(id: string, owner?: string, mount?: boolean): TemplateMeta | undefined {
  // Only mounting revives a released region, before the template cache's weak
  // copy can mask it; lookups must not pin it again.
  const template = (mount ? revive(id) : undefined) ?? getTemplate(id);
  if (!template) return undefined;
  if (owner !== undefined && (template.f !== id || template.o !== owner)) {
    throw new Error(`[WebUI] Template fragment "${id}" does not belong to <${owner}>. Reload matching resources.`);
  }
  if (!owners.has(template)) {
    owners.set(template, template);
    const blocks = template.b;
    if (blocks) for (let i = 0; i < blocks.length; i++) owners.set(blocks[i], template);
  }
  return template;
}

/** Reuse a released region template that is still alive instead of refetching it. */
function revive(id: string): TemplateMeta | undefined {
  const entry = released.get(id);
  if (!entry) return undefined;
  released.delete(id);
  const template = entry.template.deref();
  if (!template) return undefined;
  const runtime = window.__webui ??= {};
  (runtime.templates ??= {})[id] = template;
  registered.set(id, { url: entry.url, bit: entry.bit, epoch: runtime.inventoryEpoch ?? 0 });
  if (entry.bit !== undefined) runtime.inventory = writeBit(runtime.inventory ?? '', entry.bit, true);
  return template;
}

/**
 * Load a region's pack and the scripts of its custom elements in parallel.
 *
 * Every caller shares one pack request until it installs, and one load per
 * custom element. The raw pack path stays the key; only the request resolves
 * it against the asset base. In link builds `styles` names the stylesheets the
 * pack registers, which every load of the pack starts at its own priority;
 * the pack's style registration joins those requests.
 *
 * A bundler's `import()` takes no fetch priority, so a `low` load before the
 * window has loaded fetches only the pack and imports the scripts once it
 * has; `styles.js` holds its stylesheets the same way. An activation starts
 * both at once, also when it joins such a load.
 */
export function load(
  region: string,
  url: string,
  priority: Priority = 'auto',
  styles?: readonly string[],
): Promise<unknown> {
  let scripts: Promise<unknown> | undefined;
  if (priority !== 'low' || document.readyState === 'complete') scripts = loadScripts(region);
  else window.addEventListener('load', () => loadScripts(region)?.catch(noop), { once: true });
  // An installing pack has published its templates before their definitions
  // and resources are ready, so every caller joins the in-flight load. A
  // failed install may have published templates whose resources never became ready.
  let pack = loads.get(url);
  if (!pack && (unready.has(url) || !resolve(region, undefined, true))) {
    // A stalled request must settle so the next activation can retry it.
    loads.set(url, pack = fetch(resolveAsset(url), {
      priority, signal: AbortSignal.timeout?.(PACK_TIMEOUT_MS),
    } as RequestInit)
      .then(response => {
        if (!response.ok) {
          throw new Error(`[WebUI] Lazy region pack ${url} failed with HTTP ${response.status}.`);
        }
        return response.json() as Promise<TemplatePack>;
      })
      .then(pack => install(pack, region, url, priority))
      .finally(() => { loads.delete(url); }));
  }
  if (pack && styles) window.__webuiPreloadStyles?.(styles, priority);
  return Promise.all([pack, scripts]);
}

/** Whether a pack has not finished installing, so its region cannot mount yet. */
export function installing(url: string): boolean {
  return loads.has(url) || unready.has(url);
}

/**
 * Whether an active region still needs its pack or scripts before it mounts.
 *
 * A pack that is still installing has published templates that are not ready,
 * and a body's custom elements must be defined before it is inserted.
 */
export function unloaded(region: string, url: string, owner: string): boolean {
  return installing(url) || !resolve(region, owner, true) || !!loadScripts(region)?.catch(noop);
}

/** Register a pack's missing templates; resolves once they can mount. */
async function install(pack: TemplatePack, region: string, url: string, priority: Priority): Promise<void> {
  if (pack?.type !== 'webui-pack' || pack.version !== 1 || pack.fragment !== region) throw mismatch(url, region);
  const runtime = window.__webui ??= {};
  const registry = getTemplateRegistry();
  const templates = pack.templates ?? {};
  let fresh: Record<string, TemplateMeta> | undefined;
  let closures = '';
  for (const id in templates) {
    // Packs are coarse; the page may already own shared templates. Ones only
    // the template cache still holds weakly are registered again, so the
    // merged inventory never names a template the page can lose. A failed
    // install's templates are registered again so their readiness reruns.
    if (
      !unready.get(url)?.has(id)
      && (registry?.[id] || revive(id) || window.__webuiRetainTemplate?.(id))
    ) continue;
    (fresh ??= {})[id] = templates[id];
    // Registering prepares a template's own stylesheets on paths that pass
    // styles.js only the template, so the template carries the speculation.
    if (priority === 'low') speculativeTemplates.add(templates[id]);
    const source = pack.templateFunctions?.[id];
    if (source) closures += `f[${JSON.stringify(id)}]=${source};`;
  }
  // Published templates are present but not ready until every step below
  // succeeds. Only their IDs are kept, so template collection can still free them.
  unready.set(url, new Set(Object.keys(fresh || 0)));
  const waits: PromiseLike<unknown>[] = [];
  if (fresh) {
    if (closures) {
      runCompilerScript(
        `(function(){var w=window.__webui||(window.__webui={}),f=w.templateFns||(w.templateFns={});${closures}})();`,
        runtime.nonce
          ?? document.querySelector<HTMLMetaElement>('meta[name="webui-nonce"]')?.content ?? '',
      );
    }
    installTemplateRegistry();
    window.dispatchEvent(new CustomEvent(TEMPLATES_REGISTERED_EVENT, {
      detail: {
        templates: fresh,
        componentStyles: pack.componentStyles,
        waitUntil: (ready: PromiseLike<unknown>) => { waits.push(ready); },
      },
    }));
    // Registration failures surface here rather than as a missing region.
    if (waits.length > 0) await Promise.all(waits);
    if (typeof __WEBUI_DEV__ === 'undefined' || __WEBUI_DEV__) {
      warnUnscriptedComponents(region, fresh, hasPendingTemplateDefinition);
    }
    if (fresh[region]) {
      registered.set(region, { url, bit: pack.bits?.[region], epoch: runtime.inventoryEpoch ?? 0 });
    }
  }
  if (!resolve(region)) throw mismatch(url, region);
  // Every template a pack names is present now, so its bits are safe to merge.
  runtime.inventory = mergeBits(runtime.inventory ?? '', pack.inventory ?? '');
  runtime.styleInventory = mergeBits(runtime.styleInventory ?? '', pack.styleInventory ?? '');
  // `styles.js` returns nothing to wait for while it holds a speculative
  // install's stylesheets for the window load event.
  await window.__webuiPrepareResources?.(pack.required ?? [], priority);
  unready.delete(url);
}

function mismatch(url: string, region: string): Error {
  return new Error(
    `[WebUI] ${url} is not the template pack for lazy region "${region}". Serve packs from the same build as the page.`,
  );
}

/** Load an active region, then let its host re-evaluate it; a closed region stays unmounted. */
export function activate(host: LazyHost, binding: CondBinding, url: string, styles?: readonly string[]): void {
  if (!pending.get(host)?.get(binding)) {
    track(host, binding, load(binding.blockIndex as string, url, 'high', styles));
  }
}

/** Load the scripts of a region whose templates arrived with its open body. */
export function hydrated(host: LazyHost, region: string): void {
  if (pending.get(host)?.get(region)) return;
  const scripts = loadScripts(region);
  if (scripts) track(host, region, scripts);
}

/**
 * Record work that an active region waits for: a binding's activation, which
 * re-enters its host once loaded, or the scripts of a body that hydrated open.
 * A failure is reported unless a `lazyerror` listener handles it.
 */
function track(host: LazyHost, key: CondBinding | string, work: Promise<unknown>): void {
  const entries = pending.get(host) ?? new Map<CondBinding | string, Promise<void> | undefined>();
  pending.set(host, entries);
  // Only one load per key is in flight, so each settles its own entry.
  const loading: Promise<void> = work.then(() => {
    entries.delete(key);
    if (typeof key !== 'string' && attached(key)) host.$toggleCond(key);
  }, (error: unknown) => {
    entries.set(key, undefined);
    if (host.dispatchEvent(new CustomEvent('lazyerror', {
      bubbles: true, composed: true, cancelable: true, detail: { error },
    }))) {
      (globalThis.reportError ?? console.error)(error);
    }
    throw error;
  });
  // Only `$rendered()` callers observe the rejection.
  loading.catch(noop);
  entries.set(key, loading);
}

/** Wait for the host's active regions, including nested ones they activate; retry failures. */
export async function rendered(host: LazyHost): Promise<void> {
  const entries = pending.get(host);
  while (entries && entries.size > 0) {
    const waits: Promise<void>[] = [];
    for (const [key, loading] of entries) {
      if (!loading) {
        if (typeof key === 'string') hydrated(host, key);
        else if (attached(key)) host.$toggleCond(key);
      }
      const next = entries.get(key);
      if (next) waits.push(next);
      else entries.delete(key);
    }
    if (waits.length === 0) return;
    await Promise.all(waits);
  }
}

/** Disposing an instance empties its bindings, so a pack arriving later never mounts into it. */
function attached(binding: CondBinding): boolean {
  return binding.owner.conds.includes(binding);
}

/**
 * The hostless template that owns a block, if any.
 *
 * Every block passes here as it renders or hydrates, which arms the prefetch
 * triggers of the lazy regions it contains.
 */
export function blockOwner(meta: TemplateBlockMeta): TemplateMeta | undefined {
  if ((meta as TemplateMeta).lp && !scanned.has(meta as TemplateMeta)) scan(meta as TemplateMeta);
  const regions = armable.get(meta);
  if (regions) arm(regions);
  return owners.get(meta);
}

/** Visit each lazy region whose condition a template or one of its local blocks contains. */
function regions(
  template: TemplateMeta | undefined,
  visit: (block: TemplateBlockMeta, entry: CompiledConditionalMeta, url: string, styles?: readonly string[]) => void,
): void {
  const packs = template?.lp;
  if (!packs) return;
  const blocks = template!.b;
  for (let i = -1; i < (blocks?.length ?? 0); i++) {
    const block = i < 0 ? template! : blocks![i];
    const conds = block.c;
    // Numeric block references never name a pack.
    if (conds) {
      for (let j = 0; j < conds.length; j++) {
        const region = conds[j][1];
        if (packs[region]) visit(block, conds[j], packs[region], template!.lc?.[region]);
      }
    }
  }
}

/** Index a template's triggered regions by the block that contains each condition. */
function scan(template: TemplateMeta): void {
  scanned.add(template);
  regions(template, (block, [, region, , trigger], url, styles) => {
    if (!trigger || trigger === DEMAND) return;
    let armed = armable.get(block);
    if (!armed) armable.set(block, armed = []);
    armed.push([region as string, url, trigger, styles]);
  });
}

/**
 * Prefetch the regions whose conditions a component's own template contains,
 * including demand regions, or only those whose condition reads `path` or a
 * path below it.
 */
export function prefetchRegions(template: TemplateMeta | undefined, path?: string): void {
  regions(template, (_block, [[, paths], region], url, styles) => {
    if (path === undefined || paths.some(read => read.startsWith(path) && (read[path.length] ?? '.') === '.')) {
      prefetch(region as string, url, styles);
    }
  });
}

function arm(regions: readonly Armed[]): void {
  for (let i = 0; i < regions.length; i++) {
    const armed = regions[i];
    const [region, url, trigger, styles] = armed;
    if (resolve(region)) continue;
    if (trigger === INTENT) {
      if (!intent) {
        intent = new Map();
        for (const type of INTENT_EVENTS) {
          document.addEventListener(type, onIntent, { capture: true, passive: true });
        }
      }
      // Region identities start with their owner's tag.
      const owner = region.slice(0, region.indexOf('::'));
      const waiting = intent.get(owner);
      if (waiting) waiting.set(url, armed);
      // Focus that entered the owner before anything listened, such as
      // autofocus, is intent that no event will report.
      else if (focusWithin(owner)) prefetch(region, url, styles);
      else intent.set(owner, new Map([[url, armed]]));
    } else if (
      // A region shown and released before is not speculatively pinned again.
      !released.has(region)
      // Speculative page-level prefetch yields to the user's data-saving preference.
      && !(navigator as { connection?: { saveData?: boolean } }).connection?.saveData
    ) {
      speculate(armed);
    }
  }
}

/** Whether focus is inside an element with this tag, following open shadow roots. */
function focusWithin(tag: string): boolean {
  let focused: Element | null | undefined = document.activeElement;
  for (; focused; focused = focused.shadowRoot?.activeElement) if (focused.closest(tag)) return true;
  return false;
}

function speculate(armed: Armed): void {
  const trigger = armed[2];
  let queue = queues.get(trigger);
  if (!queue) {
    const waiting = queue = new Map();
    queues.set(trigger, waiting);
    const flush = (): void => {
      queues.delete(trigger);
      prefetchAll(waiting, true);
    };
    // Eager prefetch runs after the current hydration task; idle prefetch once the loaded page is idle.
    if (trigger === EAGER) setTimeout(flush, 0);
    else afterLoad(flush);
  }
  queue.set(armed[1], armed);
}

/** One delegated listener prefetches the waiting regions of every owner on the event path. */
function onIntent(event: Event): void {
  const path = event.composedPath();
  for (let i = 0; i < path.length; i++) {
    const owner = (path[i] as Partial<Element>).localName;
    const regions = owner ? intent!.get(owner) : undefined;
    if (!regions) continue;
    intent!.delete(owner!);
    prefetchAll(regions);
  }
}

function prefetchAll(regions: Map<string, Armed>, background?: boolean): void {
  for (const [region, url, , styles] of regions.values()) prefetch(region, url, styles, background);
}

/**
 * Load a region speculatively, never while the page is prerendering.
 *
 * Background (eager and idle) loads always use low fetch priority. Intent and
 * explicit prefetch stay low until the window has loaded so they never
 * compete with the critical path. Failures are silent: activation reports
 * them and retries.
 */
function prefetch(region: string, url: string, styles?: readonly string[], background?: boolean): void {
  if ((document as { prerendering?: boolean }).prerendering) {
    document.addEventListener('prerenderingchange', () => prefetch(region, url, styles, background), { once: true });
  } else {
    load(region, url, background || document.readyState !== 'complete' ? 'low' : 'high', styles).catch(noop);
  }
}

/** Remember a binding that mounted a region this runtime registered. */
export function opened(region: string, binding: CondBinding): void {
  if (!registered.has(region)) return;
  // A reopened binding may appear twice until the next idle check prunes closed ones.
  const ref = new WeakRef(binding);
  const bindings = views.get(region);
  if (bindings) bindings.add(ref);
  else views.set(region, new Set([ref]));
}

/** After instances are removed, check at idle for regions no binding still shows. */
export function disposed(): void {
  if (releasing || views.size === 0) return;
  releasing = true;
  idle(releaseInactive);
}

/** Drop unmounted region templates; a reopened region revives or refetches them. */
function releaseInactive(): void {
  releasing = false;
  const runtime = window.__webui;
  for (const [region, bindings] of views) {
    // Closing or disposing a view clears its binding's instance.
    for (const ref of bindings) if (!ref.deref()?.instance) bindings.delete(ref);
    if (bindings.size > 0) continue;
    views.delete(region);
    const record = registered.get(region);
    if (!record) continue;
    registered.delete(region);
    const template = runtime?.templates?.[region];
    // Template collection already reset this epoch's registry and inventory.
    if (!runtime || !template || record.epoch !== (runtime.inventoryEpoch ?? 0)) continue;
    delete runtime.templates![region];
    released.set(region, { ...record, template: new WeakRef(template) });
    if (record.bit !== undefined) runtime.inventory = writeBit(runtime.inventory ?? '', record.bit, false);
  }
}

function idle(callback: () => void): void {
  if (typeof requestIdleCallback === 'function') requestIdleCallback(callback, { timeout: 5000 });
  else setTimeout(callback, 200);
}

/** Run once the window has loaded and the browser is idle. */
export function afterLoad(callback: () => void): void {
  if (document.readyState === 'complete') idle(callback);
  else window.addEventListener('load', () => idle(callback), { once: true });
}

function mergeBits(current: string, next: string): string {
  if (!next) return current;
  let merged = '';
  for (let i = 0; i < current.length || i < next.length; i++) {
    merged += (parseInt(current[i] || '0', 16) | parseInt(next[i] || '0', 16)).toString(16);
  }
  return merged;
}

/** Byte `bit >> 3` is two hex digits, high nibble first; bit 0 is the low nibble's 1. */
function writeBit(inventory: string, bit: number, set: boolean): string {
  const index = (bit >> 3) * 2 + ((bit & 7) < 4 ? 1 : 0);
  const text = inventory.padEnd(index + (index & 1 ? 1 : 2), '0');
  const digit = parseInt(text[index], 16);
  const mask = 1 << (bit & 3);
  return text.slice(0, index) + (set ? digit | mask : digit & ~mask).toString(16) + text.slice(index + 1);
}
