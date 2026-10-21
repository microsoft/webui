// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { acceptInventory, acceptStyleInventory, inventoryEpoch } from './inventory.js';
import { registerTemplatesAndStyles, waitForTemplateReadiness } from './templates.js';
import { loadRuntimeBootstrap } from './runtime-bootstrap.js';
import { abortable } from './abortable.js';
import {
  deferredFragment, deferredRecord, jsonDeferredTransport,
  type DeferredReadContext, type DeferredRecord, type DeferredTransport,
} from './deferred-data.js';

/** Resource loading policy; no renderer or router is installed by this module. */
export interface DeferredLoaderOptions {
  /** Resource-only endpoint. Defaults to /_webui/deferred. */
  endpoint?: string;
  /** Resource id -> optional behavior import. Fragment bodies are not components. */
  loaders?: Record<string, () => Promise<unknown>>;
  /** JSON by default; import deferred-ndjson.js to select NDJSON explicitly. */
  transport?: DeferredTransport;
  /** Whole-request deadline, including deferred state. Defaults to 10 seconds. */
  timeoutMs?: number;
}

/** The application's state endpoint and instance-local cancellation signal. */
export interface DeferredFetchOptions {
  url: string;
  signal?: AbortSignal;
}

/** Shared resource preload with independent, uncached state requests. */
export class DeferredLoader {
  private readonly pending = new Map<string, { epoch: number; promise: Promise<void> }>();
  private readonly prepared = new Map<string, number>();
  private readonly modules = new Map<string, Promise<void>>();
  private readonly timeoutMs: number;
  private epoch = -1;

  constructor(private readonly options: DeferredLoaderOptions = {}) {
    this.timeoutMs = options.timeoutMs ?? 10_000;
    if (!Number.isFinite(this.timeoutMs) || this.timeoutMs <= 0) {
      throw new RangeError('[WebUI] Deferred timeoutMs must be positive and finite.');
    }
  }

  /** Preload an ordinary resource closure without fetching state or creating DOM. */
  async ensureLoaded(owner: string, id: string, signal?: AbortSignal): Promise<void> {
    signal?.throwIfAborted();
    loadRuntimeBootstrap();
    const fragment = deferredFragment(owner, id);
    for (let attempt = 0; attempt < 2; attempt++) {
      const epoch = this.currentEpoch();
      if (this.prepared.get(fragment) === epoch) return;
      let pending = this.pending.get(fragment);
      if (!pending || pending.epoch !== epoch) {
        const entry = { epoch, promise: Promise.resolve() };
        entry.promise = this.preload(owner, id, fragment, epoch).finally(() => {
          if (this.pending.get(fragment) === entry) this.pending.delete(fragment);
        });
        this.pending.set(fragment, entry);
        pending = entry;
      }
      if (!await waitForTemplateReadiness(pending.promise, signal)) signal?.throwIfAborted();
      if (epoch === inventoryEpoch()) return;
    }
    throw new Error('[WebUI] Resources were repeatedly collected during preload. Retry after collection finishes.');
  }

  /** Fetch fresh scoped state; the caller applies it to the existing owner. */
  async fetchPartial<State extends Record<string, unknown> = Record<string, unknown>>(
    owner: string,
    id: string,
    options: DeferredFetchOptions,
  ): Promise<State> {
    loadRuntimeBootstrap();
    const fragment = deferredFragment(owner, id);
    const controller = new AbortController();
    const signal = options.signal ? AbortSignal.any([options.signal, controller.signal]) : controller.signal;
    signal.throwIfAborted();
    const timer = setTimeout(() => controller.abort(new DOMException('Deferred response timed out', 'TimeoutError')), this.timeoutMs);
    const transport = this.options.transport ?? jsonDeferredTransport;
    let failure: unknown;
    try {
      const initialModule = this.module(fragment);
      if (initialModule) void initialModule.catch(error => { failure = error; controller.abort(error); });
      for (let attempt = 0; attempt < 2; attempt++) {
        const epoch = this.currentEpoch();
        const response = await fetch(options.url, { headers: this.headers(transport.accept), signal });
        if (!response.ok) throw new Error(`[WebUI] Deferred request failed: ${response.status} ${response.statusText}`);
        const contentType = response.headers.get('content-type') ?? '';
        if (!contentType.includes('json')) throw new Error('[WebUI] Deferred state endpoints must return JSON or NDJSON.');
        const context: DeferredReadContext = {
          owner, fragment, signal,
          abort: reason => controller.abort(reason),
          prepare: record => this.prepare(record, epoch),
        };
        const state = await abortable(transport.read(response, context), signal);
        if (initialModule) await abortable(initialModule, signal);
        signal.throwIfAborted();
        if (epoch === inventoryEpoch()) return state as State;
      }
      throw new Error('[WebUI] Resources were repeatedly collected during loading. Retry after collection finishes.');
    } catch (error) {
      throw failure ?? error;
    } finally {
      clearTimeout(timer);
    }
  }

  private currentEpoch(): number {
    const current = inventoryEpoch();
    if (current !== this.epoch) {
      this.epoch = current;
      this.prepared.clear();
    }
    return current;
  }

  private headers(accept: string): Record<string, string> {
    const runtime = window.__webui;
    const headers: Record<string, string> = { Accept: accept };
    if (runtime?.inventory) headers['X-WebUI-Inventory'] = runtime.inventory;
    if (runtime?.styleInventory) headers['X-WebUI-Style-Inventory'] = runtime.styleInventory;
    if (runtime?.buildId) headers['X-WebUI-Build'] = runtime.buildId;
    return headers;
  }

  private async preload(owner: string, id: string, fragment: string, epoch: number): Promise<void> {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(new DOMException('Deferred preload timed out', 'TimeoutError')), this.timeoutMs);
    let failure: unknown;
    try {
      const initialModule = this.module(fragment);
      if (initialModule) void initialModule.catch(error => { failure = error; controller.abort(error); });
      const existing = window.__webuiResolveResources?.(fragment);
      if (existing) {
        await abortable(this.prepareRequired(existing), controller.signal);
        if (epoch === inventoryEpoch()) this.prepared.set(fragment, epoch);
        return;
      }
      const url = new URL(this.options.endpoint ?? '/_webui/deferred', window.location.href);
      url.searchParams.set('owner', owner);
      url.searchParams.set('id', id);
      const response = await fetch(url, { headers: this.headers('application/json'), signal: controller.signal });
      if (!response.ok) throw new Error(`[WebUI] Deferred preload failed: ${response.status} ${response.statusText}`);
      if (!response.headers.get('content-type')?.includes('application/json')) {
        throw new Error('[WebUI] Deferred resource endpoints must return JSON.');
      }
      const record = deferredRecord(await response.json(), { owner, fragment });
      if (record.state !== undefined) throw new Error('[WebUI] Resource preloading must not fetch application state.');
      if (!await waitForTemplateReadiness(this.prepare(record, epoch), controller.signal)) controller.signal.throwIfAborted();
      if (initialModule) await abortable(initialModule, controller.signal);
      controller.signal.throwIfAborted();
    } catch (error) {
      throw failure ?? error;
    } finally {
      clearTimeout(timer);
    }
  }

  private prepare(record: DeferredRecord, epoch: number): Promise<void> {
    // The caller retries with fresh inventory rather than restoring an evicted batch.
    if (epoch !== inventoryEpoch()) return Promise.resolve();
    const expected = window.__webui?.buildId;
    if (expected && expected !== record.buildId) {
      throw new Error('[WebUI] Deferred resources belong to a different build. Reload the document.');
    }
    const runtime = window.__webui ??= {};
    if (!expected) {
      const templates = Reflect.get(runtime, 'templates') as Record<string, unknown> | undefined;
      if (templates && Object.keys(templates).length > 0) {
        throw new Error('[WebUI] Existing templates have no artifact identity. Rebuild the initial document.');
      }
      runtime.buildId = record.buildId;
    }
    const available = Reflect.get(runtime, 'templates') as Record<string, unknown> | undefined;
    for (const id of record.required) {
      if (!(record.templates && Object.hasOwn(record.templates, id)) &&
          !(available && Object.hasOwn(available, id)) && !window.__webuiRetainTemplate?.(id)) {
        throw new Error(`[WebUI] Deferred response did not supply required resource "${id}". Retry without stale inventory.`);
      }
    }
    if (!record.templates && !record.componentStyles && !record.templateFunctions &&
        record.required.every(id => this.prepared.get(id) === epoch)) {
      acceptInventory(record.inventory, epoch);
      acceptStyleInventory(record.styleInventory, epoch);
      return Promise.resolve();
    }
    const ready = registerTemplatesAndStyles(
      record, runtime.nonce ?? '',
      (inventory, styles) => {
        acceptInventory(inventory, epoch);
        if (styles !== undefined) acceptStyleInventory(styles, epoch);
      },
    );
    const modules = this.prepareRequired(record.required);
    const fragment = record.fragment;
    const required = record.required;
    return Promise.all([ready, modules]).then(() => {
      if (epoch !== inventoryEpoch()) return;
      this.prepared.set(fragment, epoch);
      for (let i = 0; i < required.length; i++) this.prepared.set(required[i], epoch);
    });
  }

  private async prepareRequired(required: readonly string[]): Promise<void> {
    const work: Promise<void>[] = [];
    for (let i = 0; i < required.length; i++) {
      const pending = this.module(required[i]);
      if (pending) work.push(pending);
    }
    const resources = window.__webuiPrepareResources?.(required);
    if (resources) work.push(resources);
    await Promise.all(work);
  }

  private module(id: string): Promise<void> | undefined {
    const load = this.options.loaders?.[id];
    if (!load) return undefined;
    let pending = this.modules.get(id);
    if (!pending) {
      pending = Promise.resolve().then(load).then(() => {});
      this.modules.set(id, pending);
      void pending.catch(() => {
        if (this.modules.get(id) === pending) this.modules.delete(id);
      });
    }
    return pending;
  }
}

/** Create a loader without starting routing or installing a hydration runtime. */
export function createDeferredLoader(options?: DeferredLoaderOptions): DeferredLoader {
  return new DeferredLoader(options);
}
