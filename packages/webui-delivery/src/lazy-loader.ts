// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { acceptInventory, acceptStyleInventory, inventoryEpoch } from './inventory.js';
import { registerTemplatesAndStyles, waitForTemplateReadiness } from './templates.js';
import { loadRuntimeBootstrap } from './runtime-bootstrap.js';
import { abortable } from './abortable.js';
import { ResourceRequest, resourceHeaders, validateTimeout, DEFAULT_TIMEOUT_MS } from './request.js';
import { jsonTransport, type PartialTransport } from './transport.js';
import {
  lazyFragment, lazyRecord, readLazyBody,
  type LazyReadContext, type LazyRecord,
} from './lazy-data.js';

/** Resource loading policy; no renderer or router is installed by this module. */
export interface LazyLoaderOptions {
  /** Resource-only endpoint. Defaults to /_webui/lazy. */
  endpoint?: string;
  /** Resource id -> optional behavior import. Fragment bodies are not components. */
  loaders?: Record<string, () => Promise<unknown>>;
  /** JSON by default; select ndjson() from resources.js for two-phase delivery. */
  transport?: PartialTransport;
  /** Whole-request deadline, including lazy state. Defaults to 10 seconds. */
  timeoutMs?: number;
}

/** The application's state endpoint and instance-local cancellation signal. */
export interface LazyFetchOptions {
  url: string;
  signal?: AbortSignal;
}

/** Shared resource preload with independent, uncached state requests. */
export class LazyLoader {
  private readonly pending = new Map<string, { epoch: number; promise: Promise<void> }>();
  private readonly prepared = new Map<string, number>();
  private readonly modules = new Map<string, Promise<void>>();
  private readonly timeoutMs: number;
  private readonly options: LazyLoaderOptions;
  private epoch = -1;

  constructor(options: LazyLoaderOptions = {}) {
    this.options = { ...options };
    this.timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
    validateTimeout(this.timeoutMs);
  }

  /** Preload an ordinary resource closure without fetching state or creating DOM. */
  async ensureLoaded(owner: string, id: string, signal?: AbortSignal): Promise<void> {
    signal?.throwIfAborted();
    loadRuntimeBootstrap();
    const fragment = lazyFragment(owner, id);
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
    options: LazyFetchOptions,
  ): Promise<State> {
    loadRuntimeBootstrap();
    const fragment = lazyFragment(owner, id);
    const request = new ResourceRequest(options.signal, this.timeoutMs);
    const { controller, signal } = request;
    const transport = this.options.transport ?? jsonTransport;
    let failure: unknown;
    try {
      const initialModule = this.module(fragment);
      if (initialModule) void initialModule.catch(error => { failure = error; controller.abort(error); });
      for (let attempt = 0; attempt < 2; attempt++) {
        const epoch = this.currentEpoch();
        const response = await fetch(options.url, { headers: resourceHeaders(transport.accept), signal });
        if (!response.ok) throw new Error(`[WebUI] Lazy request failed: ${response.status} ${response.statusText}`);
        const contentType = response.headers.get('content-type') ?? '';
        if (!contentType.includes('json')) throw new Error('[WebUI] Lazy state endpoints must return JSON or NDJSON.');
        const context: LazyReadContext = {
          owner, fragment, signal,
          abort: reason => controller.abort(reason),
          prepare: record => this.prepare(record, epoch),
        };
        const body = await transport.read(response, signal);
        const state = await abortable(readLazyBody(body, context), signal);
        if (initialModule) await abortable(initialModule, signal);
        signal.throwIfAborted();
        if (epoch === inventoryEpoch()) return state as State;
      }
      throw new Error('[WebUI] Resources were repeatedly collected during loading. Retry after collection finishes.');
    } catch (error) {
      controller.abort(failure ?? error);
      throw failure ?? error;
    } finally {
      request.dispose();
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

  private async preload(owner: string, id: string, fragment: string, epoch: number): Promise<void> {
    const request = new ResourceRequest(undefined, this.timeoutMs);
    const { controller } = request;
    let failure: unknown;
    try {
      const initialModule = this.module(fragment);
      if (initialModule) void initialModule.catch(error => { failure = error; controller.abort(error); });
      const existing = window.__webuiResolveResources?.(fragment);
      if (existing) {
        const preparing = this.prepareRequired(existing);
        if (preparing) await abortable(preparing, controller.signal);
        if (epoch === inventoryEpoch()) this.prepared.set(fragment, epoch);
        return;
      }
      const url = new URL(this.options.endpoint ?? '/_webui/lazy', window.location.href);
      url.searchParams.set('owner', owner);
      url.searchParams.set('id', id);
      const response = await fetch(url, { headers: resourceHeaders('application/json'), signal: controller.signal });
      if (!response.ok) throw new Error(`[WebUI] Lazy preload failed: ${response.status} ${response.statusText}`);
      if (!response.headers.get('content-type')?.includes('application/json')) {
        throw new Error('[WebUI] Lazy resource endpoints must return JSON.');
      }
      const record = lazyRecord(await response.json(), { owner, fragment });
      if (record.state !== undefined) throw new Error('[WebUI] Resource preloading must not fetch application state.');
      const preparing = this.prepare(record, epoch);
      if (preparing && !await waitForTemplateReadiness(preparing, controller.signal)) controller.signal.throwIfAborted();
      if (initialModule) await abortable(initialModule, controller.signal);
      controller.signal.throwIfAborted();
    } catch (error) {
      controller.abort(failure ?? error);
      throw failure ?? error;
    } finally {
      request.dispose();
    }
  }

  private prepare(record: LazyRecord, epoch: number): Promise<void> | undefined {
    // The caller retries with fresh inventory rather than restoring an evicted batch.
    if (epoch !== inventoryEpoch()) return;
    const expected = window.__webui?.buildId;
    if (expected && expected !== record.buildId) {
      throw new Error('[WebUI] Lazy resources belong to a different build. Reload the document.');
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
        throw new Error(`[WebUI] Lazy response did not supply required resource "${id}". Retry without stale inventory.`);
      }
    }
    if (!record.templates && !record.componentStyles && !record.templateFunctions &&
        record.required.every(id => this.prepared.get(id) === epoch)) {
      acceptInventory(record.inventory, epoch);
      acceptStyleInventory(record.styleInventory, epoch);
      return;
    }
    const ready = registerTemplatesAndStyles(
      record, runtime.nonce ?? '',
      (inventory, styles) => {
        acceptInventory(inventory, epoch);
        if (styles !== undefined) acceptStyleInventory(styles, epoch);
      },
    );
    const modules = this.prepareRequired(record.required);
    const prepared = (): void => {
      if (epoch !== inventoryEpoch()) return;
      const required = record.required;
      this.prepared.set(record.fragment, epoch);
      for (let i = 0; i < required.length; i++) this.prepared.set(required[i], epoch);
    };
    const pending = ready && modules ? Promise.all([ready, modules]) : ready ?? modules;
    if (pending) return pending.then(prepared);
    prepared();
  }

  private prepareRequired(required: readonly string[]): Promise<void> | undefined {
    let work: Promise<void>[] | undefined;
    for (let i = 0; i < required.length; i++) {
      const pending = this.module(required[i]);
      if (pending) (work ??= []).push(pending);
    }
    const resources = window.__webuiPrepareResources?.(required);
    if (resources) (work ??= []).push(resources);
    return work ? Promise.all(work).then(() => {}) : undefined;
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
export function createLazyLoader(options?: LazyLoaderOptions): LazyLoader {
  return new LazyLoader(options);
}
