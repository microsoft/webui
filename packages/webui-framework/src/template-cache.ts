// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { getTemplate, getTemplateRegistry } from './template-registry.js';
import { installTemplateCacheHooks, type TemplateCacheHooks } from './template-cache-contract.js';
import type { TemplateMeta } from './template-types.js';

interface TemplateRecord {
  meta: TemplateMeta | undefined;
  dependencies: readonly TemplateRecord[];
}

interface TemplateCache {
  records: Map<string, WeakRef<TemplateRecord>>;
  ownership: WeakMap<TemplateMeta, TemplateRecord>;
  elements: WeakMap<Element, TemplateMeta | readonly TemplateMeta[]>;
  hydrated: WeakSet<Element>;
  owners: Map<string, Set<WeakRef<TemplateRecord>>>;
}

const EMPTY_DEPENDENCIES: readonly TemplateRecord[] = [];
const caches = new WeakMap<Window, TemplateCache>();

function cacheForWindow(): TemplateCache {
  let cache = caches.get(window);
  if (!cache) {
    cache = {
      records: new Map(),
      ownership: new WeakMap(),
      elements: new WeakMap(),
      hydrated: new WeakSet(),
      owners: new Map(),
    };
    caches.set(window, cache);
  }
  return cache;
}

function recordFor(cache: TemplateCache, tag: string): TemplateRecord {
  let record = cache.records.get(tag)?.deref();
  if (!record) {
    record = { meta: undefined, dependencies: EMPTY_DEPENDENCIES };
    cache.records.set(tag, new WeakRef(record));
  }
  return record;
}

function validateOwnership(tag: string, meta: TemplateMeta): void {
  if (meta.v !== 3) {
    throw new Error(
      `[WebUI] Template <${tag}> has no cache ownership metadata. ` +
      'Rebuild the complete protocol with a matching WebUI compiler before collecting templates.',
    );
  }
  if (meta.d === undefined) return;
  if (!Array.isArray(meta.d) || meta.d.some(name => typeof name !== 'string' || name.length === 0)) {
    throw new Error(`[WebUI] Invalid component dependencies for <${tag}>. Rebuild the protocol.`);
  }
}

function prepareRecord(cache: TemplateCache, tag: string, meta: TemplateMeta): TemplateRecord {
  validateOwnership(tag, meta);
  const record = recordFor(cache, tag);
  if (record.meta && record.meta !== meta && !sameTemplate(record.meta, meta)) {
    throw new Error(
      `[WebUI] Template <${tag}> conflicts with a live compiled template. ` +
      'Reload the document with matching application and protocol artifacts.',
    );
  }
  return record;
}

function commitRecord(cache: TemplateCache, record: TemplateRecord, meta: TemplateMeta): TemplateMeta {
  if (record.meta) return record.meta;
  record.meta = meta;
  const dependencies = meta.d;
  if (dependencies && dependencies.length > 0) {
    const records = new Array<TemplateRecord>(dependencies.length);
    for (let i = 0; i < dependencies.length; i++) {
      records[i] = recordFor(cache, dependencies[i]);
    }
    record.dependencies = records;
  }
  // A live metadata object owns its dependency graph, but the tag index does not.
  cache.ownership.set(meta, record);
  if (meta.f && meta.o && meta.e) {
    let owned = cache.owners.get(meta.o);
    if (!owned) cache.owners.set(meta.o, owned = new Set());
    const reference = cache.records.get(meta.f);
    if (!reference) throw new Error('[WebUI] Fragment ownership lost its resource identity.');
    owned.add(reference);
  }
  return meta;
}

const hooks: TemplateCacheHooks = {
  accept(templates) {
    const cache = cacheForWindow();
    const names = Object.keys(templates);
    const records = new Array<TemplateRecord>(names.length);
    for (let i = 0; i < names.length; i++) {
      records[i] = prepareRecord(cache, names[i], templates[names[i]]);
    }
    for (let i = 0; i < names.length; i++) {
      const name = names[i];
      templates[name] = commitRecord(cache, records[i], templates[name]);
    }
  },
  retain(tag, meta) {
    const cache = cacheForWindow();
    const existing = cache.ownership.get(meta);
    if (existing) return existing.meta ?? meta;
    return commitRecord(cache, prepareRecord(cache, tag, meta), meta);
  },
  lookup(tag) {
    return caches.get(window)?.records.get(tag)?.deref()?.meta;
  },
  hydrated(element) {
    const cache = caches.get(window);
    cache?.hydrated.add(element);
    cache?.elements.delete(element);
  },
  visitRetained(cached, visit) {
    const records = caches.get(window)?.records;
    if (!records) return;
    for (const [tag, reference] of records) {
      if (cached?.[tag]) continue;
      const meta = reference.deref()?.meta;
      if (meta) visit(tag, meta);
    }
  },
};

/** Install the optional template-cache ownership adapter before definitions run. */
export function installTemplateCache(): void {
  installTemplateCacheHooks(hooks);
  if (typeof window === 'undefined') return;
  window.__webuiCollectTemplates = collectTemplates;
  window.__webuiRetainTemplate = retainCachedTemplate;
}

function retainCachedTemplate(tag: string): TemplateMeta | undefined {
  const meta = hooks.lookup(tag);
  if (!meta) return undefined;
  if (!window.__webui) window.__webui = {};
  const templates = window.__webui.templates ??= {};
  templates[tag] = meta;
  return meta;
}

/**
 * Release cache ownership of compiled templates, retaining any requested roots.
 *
 * Mounted or otherwise retained component metadata keeps its ordinary
 * dependencies alive. SSR hosts not yet upgraded are pinned by their DOM
 * lifetime. Browser collection remains asynchronous; the return value counts
 * cache entries released, not objects already reclaimed.
 *
 * Import the template-cache entry before runtime startup or template reads. Older protocols
 * without compiler-owned dependencies fail before any cache entries are removed.
 */
export function collectTemplates(keep: readonly string[] = []): number {
  const registry = getTemplateRegistry();
  const runtime = window.__webui;
  if (!registry) {
    resetInventory();
    return 0;
  }
  const names = Object.keys(registry);
  for (let i = 0; i < names.length; i++) {
    validateOwnership(names[i], registry[names[i]]);
  }

  const cache = cacheForWindow();
  for (const [tag, reference] of cache.records) {
    if (!reference.deref()) cache.records.delete(tag);
  }
  for (const [owner, references] of cache.owners) {
    for (const reference of references) if (!reference.deref()) references.delete(reference);
    if (references.size === 0) cache.owners.delete(owner);
  }
  for (let i = 0; i < names.length; i++) getTemplate(names[i]);
  pinDocumentTemplates(cache);
  const retained = retainedRecords(cache, keep);
  let released = 0;
  for (let i = 0; i < names.length; i++) {
    const name = names[i];
    const record = cache.records.get(name)?.deref();
    if (record && retained?.has(record)) continue;
    delete registry[name];
    if (runtime?.templateFns) delete runtime.templateFns[name];
    released++;
  }
  resetInventory();
  return released;
}

function resetInventory(): void {
  const runtime = window.__webui ??= {};
  runtime.inventory = '';
  runtime.inventoryEpoch = (runtime.inventoryEpoch ?? 0) + 1;
}

function retainedRecords(
  cache: TemplateCache,
  keep: readonly string[],
): Set<TemplateRecord> | undefined {
  if (keep.length === 0) return undefined;
  const retained = new Set<TemplateRecord>();
  const pending: TemplateRecord[] = [];
  for (let i = 0; i < keep.length; i++) {
    const record = cache.records.get(keep[i])?.deref();
    if (record) pending.push(record);
  }
  while (pending.length > 0) {
    const record = pending.pop()!;
    if (retained.has(record)) continue;
    retained.add(record);
    for (let i = 0; i < record.dependencies.length; i++) {
      pending.push(record.dependencies[i]);
    }
  }
  return retained;
}

function pinDocumentTemplates(cache: TemplateCache): void {
  const roots: (Document | ShadowRoot)[] = [document];
  while (roots.length > 0) {
    const root = roots.pop()!;
    const owner = 'host' in root ? root.ownerDocument : root;
    const walker = owner.createTreeWalker(root, 1);
    let node = walker.nextNode();
    while (node) {
      const element = node as Element;
      const meta = hooks.lookup(element.localName);
      if (meta && !cache.hydrated.has(element)) {
        let pinned: TemplateMeta[] | undefined;
        const owned = cache.owners.get(element.localName);
        if (owned) {
          for (const reference of owned) {
            const fragment = reference.deref()?.meta;
            if (fragment) (pinned ??= [meta]).push(fragment);
          }
        }
        cache.elements.set(element, pinned ?? meta);
      }
      if (element.shadowRoot) roots.push(element.shadowRoot);
      node = walker.nextNode();
    }
  }
}

function sameTemplate(first: TemplateMeta, second: TemplateMeta): boolean {
  const pending: unknown[] = [first, second];
  while (pending.length > 0) {
    const right = pending.pop();
    const left = pending.pop();
    if (Object.is(left, right)) continue;
    if (typeof left === 'function' && typeof right === 'function') {
      // Compiler conditions only close over their explicit resolver/scope inputs.
      if (Function.prototype.toString.call(left) !== Function.prototype.toString.call(right)) return false;
      continue;
    }
    if (!left || !right || typeof left !== 'object' || typeof right !== 'object') return false;
    if (Array.isArray(left)) {
      if (!Array.isArray(right) || left.length !== right.length) return false;
      for (let i = 0; i < left.length; i++) pending.push(left[i], right[i]);
      continue;
    }
    if (Array.isArray(right)) return false;
    const keys = Object.keys(left);
    if (keys.length !== Object.keys(right).length) return false;
    for (let i = 0; i < keys.length; i++) {
      const key = keys[i];
      if (!Object.hasOwn(right, key)) return false;
      pending.push(Reflect.get(left, key), Reflect.get(right, key));
    }
  }
  return true;
}

/** Inspect dependency ownership without relying on garbage-collector timing. */
export function templateDependenciesForTests(meta: TemplateMeta): readonly TemplateMeta[] {
  const dependencies = caches.get(window)?.ownership.get(meta)?.dependencies;
  const templates: TemplateMeta[] = [];
  if (dependencies) {
    for (let i = 0; i < dependencies.length; i++) {
      const dependency = dependencies[i].meta;
      if (dependency) templates.push(dependency);
    }
  }
  return templates;
}
