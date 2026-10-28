// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Pack loading, prefetch triggers, activation, and release for lazy regions.
 *
 * `lazy-entry.ts` connects this module to the element. It has no element
 * dependency of its own, so it can be tested without a DOM.
 */

import { getTemplate, getTemplateRegistry, installTemplateRegistry } from './template-registry.js';
import { TEMPLATES_REGISTERED_EVENT } from './template-events.js';
import { runCompilerScript } from './trusted-types-policy.js';
import type { CondBinding } from './element/types.js';
import type { TemplateBlockMeta, TemplateMeta } from './template-types.js';

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

/** A region with a prefetch trigger: identity, pack URL, and trigger code. */
type Armed = readonly [region: string, url: string, trigger: number];

const INTENT = 1;
const IDLE = 2;
const DEMAND = 4;
const INTENT_EVENTS = ['pointerover', 'focusin', 'touchstart', 'pointerdown'];
/** Matches the delivery request deadline. */
const PACK_TIMEOUT_MS = 10_000;
const RESOLVED = Promise.resolve();
const owners = new WeakMap<TemplateBlockMeta, TemplateMeta>();
const scanned = new WeakSet<TemplateMeta>();
/** Regions with a prefetch trigger, by the block that contains their condition. */
const armable = new WeakMap<TemplateBlockMeta, Armed[]>();
const loads = new Map<string, Promise<void>>();
const registered = new Map<string, Registration>();
const released = new Map<string, Registration & { template: WeakRef<TemplateMeta> }>();
/** Bindings that mounted each region this runtime registered. */
const views = new Map<string, Set<WeakRef<CondBinding>>>();
/** Active regions per host whose packs are loading or failed to load. */
const pending = new WeakMap<Element, Set<CondBinding>>();
const activations = new WeakMap<CondBinding, Promise<void>>();
/** Intent regions waiting for interaction, by owner tag. */
let intent: Map<string, Map<string, string>> | undefined;
/** Idle and load prefetch queues, by trigger. */
const queues = new Map<number, Map<string, string>>();
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

/** Load a region's pack once; every caller shares the same request until it installs. */
export function load(region: string, url: string, priority: Priority = 'auto'): Promise<void> {
  // An installing pack has published its templates before their definitions
  // and resources are ready, so every caller joins the in-flight load.
  let loading = loads.get(url);
  if (loading) return loading;
  if (resolve(region, undefined, true)) return RESOLVED;
  // A stalled request must settle so the next activation can retry it.
  loading = fetch(url, { priority, signal: AbortSignal.timeout?.(PACK_TIMEOUT_MS) } as RequestInit)
    .then(response => {
      if (!response.ok) {
        throw new Error(`[WebUI] Lazy region pack ${url} failed with HTTP ${response.status}.`);
      }
      return response.json() as Promise<TemplatePack>;
    })
    .then(pack => install(pack, region, url))
    .finally(() => { loads.delete(url); });
  loads.set(url, loading);
  return loading;
}

/** Whether a pack is still loading or installing, so its region cannot mount yet. */
export function installing(url: string): boolean {
  return loads.has(url);
}

/** Register a pack's missing templates; resolves once they can mount. */
async function install(pack: TemplatePack, region: string, url: string): Promise<void> {
  if (pack?.type !== 'webui-pack' || pack.version !== 1 || pack.fragment !== region) throw mismatch(url, region);
  const runtime = window.__webui ??= {};
  const registry = getTemplateRegistry();
  const templates = pack.templates ?? {};
  let fresh: Record<string, TemplateMeta> | undefined;
  let closures = '';
  for (const id in templates) {
    // Packs are coarse; the page may already own shared templates. Ones only
    // the template cache still holds weakly are registered again, so the
    // merged inventory never names a template the page can lose.
    if (registry?.[id] || revive(id) || window.__webuiRetainTemplate?.(id)) continue;
    (fresh ??= {})[id] = templates[id];
    const source = pack.templateFunctions?.[id];
    if (source) closures += `f[${JSON.stringify(id)}]=${source};`;
  }
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
    if (fresh[region]) {
      registered.set(region, { url, bit: pack.bits?.[region], epoch: runtime.inventoryEpoch ?? 0 });
    }
  }
  if (!resolve(region)) throw mismatch(url, region);
  // Every template a pack names is present now, so its bits are safe to merge.
  runtime.inventory = mergeBits(runtime.inventory ?? '', pack.inventory ?? '');
  runtime.styleInventory = mergeBits(runtime.styleInventory ?? '', pack.styleInventory ?? '');
  await window.__webuiPrepareResources?.(pack.required ?? []);
}

function mismatch(url: string, region: string): Error {
  return new Error(
    `[WebUI] ${url} is not the template pack for lazy region "${region}". Serve packs from the same build as the page.`,
  );
}

/** Load an active region, then let its host re-evaluate it; a closed region stays unmounted. */
export function activate(host: LazyHost, binding: CondBinding, url: string): void {
  if (activations.has(binding)) return;
  const loading: Promise<void> = load(binding.blockIndex as string, url, 'high').then(() => {
    if (activations.get(binding) !== loading) return;
    activations.delete(binding);
    pending.get(host)?.delete(binding);
    if (attached(binding)) host.$toggleCond(binding);
  }, (error: unknown) => {
    if (activations.get(binding) === loading) activations.delete(binding);
    throw error;
  });
  activations.set(binding, loading);
  let bindings = pending.get(host);
  if (!bindings) pending.set(host, bindings = new Set());
  bindings.add(binding);
}

/** Wait for the host's active regions, including nested ones they activate; retry failures. */
export async function rendered(host: LazyHost): Promise<void> {
  const bindings = pending.get(host);
  while (bindings && bindings.size > 0) {
    const waits: Promise<void>[] = [];
    for (const binding of bindings) {
      if (!activations.has(binding) && attached(binding)) host.$toggleCond(binding);
      const loading = activations.get(binding);
      if (loading) waits.push(loading);
      else bindings.delete(binding);
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
  const packs = (meta as TemplateMeta).lp;
  if (packs && !scanned.has(meta as TemplateMeta)) scan(meta as TemplateMeta, packs);
  const regions = armable.get(meta);
  if (regions) arm(regions);
  return owners.get(meta);
}

/** Index a template's triggered regions by the block that contains each condition. */
function scan(template: TemplateMeta, packs: Readonly<Record<string, string>>): void {
  scanned.add(template);
  const blocks = template.b;
  for (let i = -1; i < (blocks?.length ?? 0); i++) {
    const block = i < 0 ? template : blocks![i];
    const conds = block.c;
    let regions: Armed[] | undefined;
    if (conds) {
      for (let j = 0; j < conds.length; j++) {
        const [, region, , trigger] = conds[j];
        const url = trigger && trigger !== DEMAND && typeof region === 'string' ? packs[region] : undefined;
        if (url) (regions ??= []).push([region as string, url, trigger!]);
      }
    }
    if (regions) armable.set(block, regions);
  }
}

function arm(regions: readonly Armed[]): void {
  for (let i = 0; i < regions.length; i++) {
    const [region, url, trigger] = regions[i];
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
      if (waiting) waiting.set(url, region);
      else intent.set(owner, new Map([[url, region]]));
    } else if (
      // A region shown and released before is not speculatively pinned again.
      !released.has(region)
      // Speculative page-level prefetch yields to the user's data-saving preference.
      && !(navigator as { connection?: { saveData?: boolean } }).connection?.saveData
    ) {
      speculate(trigger, url, region);
    }
  }
}

function speculate(trigger: number, url: string, region: string): void {
  let queue = queues.get(trigger);
  if (!queue) {
    const waiting = queue = new Map();
    queues.set(trigger, waiting);
    const flush = (): void => {
      queues.delete(trigger);
      prefetchAll(waiting, trigger === IDLE ? 'low' : 'auto');
    };
    // Load prefetch runs after the current hydration task; idle prefetch after the page loads.
    if (trigger !== IDLE) setTimeout(flush, 0);
    else if (document.readyState === 'complete') idle(flush);
    else window.addEventListener('load', () => idle(flush), { once: true });
  }
  queue.set(url, region);
}

/** One delegated listener prefetches the waiting regions of every owner on the event path. */
function onIntent(event: Event): void {
  const path = event.composedPath();
  for (let i = 0; i < path.length; i++) {
    const owner = (path[i] as Partial<Element>).localName;
    const regions = owner ? intent!.get(owner) : undefined;
    if (!regions) continue;
    intent!.delete(owner!);
    prefetchAll(regions, 'high');
  }
}

function prefetchAll(regions: Map<string, string>, priority: Priority): void {
  // Activation reports failures and retries.
  for (const [url, region] of regions) load(region, url, priority).catch(() => {});
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
