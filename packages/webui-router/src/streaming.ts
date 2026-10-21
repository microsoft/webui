// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { NdjsonReader } from './ndjson-reader.js';
import { hasState, mountedRouteComponent } from './route-element.js';
import { isStateful } from './types.js';
import {
  injectCssLinks,
  registerTemplatesAndStyles,
  waitForTemplateReadiness,
} from './templates.js';
import {
  attachDeferredStream,
  releaseDeferredStream,
  type StreamingPartialResponse,
} from './partial-response.js';
import type { RouteChainEntry } from './cache.js';
import type { RouteStreaming } from './capabilities.js';

/** Explicit NDJSON route capability; importing it readies the decoder before navigation. */
export const ndjson: RouteStreaming = { read: readStreamingPartial };

export {
  cancelDeferredStream,
  hasDeferredStream,
  startDeferredStream,
} from './partial-response.js';
export type { StreamingPartialResponse } from './partial-response.js';

/** Route-specific ownership and commit policy for a two-phase partial. */
export interface StreamingContext {
  readonly navGeneration: number;
  readonly currentRequestPath: string;
  readonly activeChain: RouteChainEntry[];
  readonly nonce: string;
  readonly injectedCss: Set<string>;
  setDeferredReader(reader: Promise<void> | null): void;
  setDeferredGeneration(gen: number): void;
  updateInventory(inv: string, styleInventory?: string): void;
  markCacheComplete(requestPath: string): void;
}

/**
 * Prepare the resource/chain record and leave deferred state paused until commit.
 * The record reader owns framing; this adapter owns route state and generations.
 */
export async function readStreamingPartial(
  response: Response,
  requestPath: string,
  context: StreamingContext,
  signal?: AbortSignal,
): Promise<StreamingPartialResponse | null> {
  if (!response.body) throw new Error('[Router] NDJSON response has no body.');
  const records = new NdjsonReader(response.body, { signal });
  const generation = context.navGeneration;
  try {
    const data = readMetadata(await records.read());
    let receivedState = data.state !== undefined;
    let buffered = records.readBuffered();
    while (buffered !== undefined) {
      persistDeferredStates(data, readStates(buffered));
      receivedState = true;
      buffered = records.readBuffered();
    }

    const ready = registerTemplatesAndStyles(
      data, context.nonce, context.injectedCss, (inventory, styles) => context.updateInventory(inventory, styles),
    );
    injectCssLinks(data, context.injectedCss);
    if (
      (ready && !await waitForTemplateReadiness(ready, signal)) ||
      signal?.aborted ||
      generation !== context.navGeneration
    ) {
      await records.cancel();
      return null;
    }

    let started = false;
    attachDeferredStream(data, {
      async cancel() {
        started = true;
        await records.cancel();
        context.setDeferredReader(null);
      },
      start() {
        if (started) return;
        started = true;
        Reflect.deleteProperty(data, '_deferredStream');
        context.setDeferredGeneration(generation);
        const pending = continueDeferredRead(
          records, data, requestPath, { context, generation, signal, receivedState },
        ).catch((error: unknown) => {
          if (!signal?.aborted && generation === context.navGeneration) {
            console.warn('[Router] Deferred state reader failed:', error);
          }
        }).finally(() => {
          releaseDeferredStream(data);
          context.setDeferredReader(null);
        });
        context.setDeferredReader(pending);
      },
    });
    return data;
  } catch (error) {
    await records.cancel(error);
    if (signal?.aborted) return null;
    throw error;
  }
}

function readMetadata(value: unknown): StreamingPartialResponse {
  if (!value || typeof value !== 'object' || !('chain' in value) || !Array.isArray(value.chain)) {
    throw new Error('[Router] NDJSON must start with a route-chain/resource record.');
  }
  return value as StreamingPartialResponse;
}

function readStates(value: unknown): (Record<string, unknown> | null)[] {
  if (!value || typeof value !== 'object') {
    throw new Error('[Router] Expected a deferred-state object in the NDJSON response.');
  }
  if ('error' in value) {
    throw new Error('[Router] The server could not complete deferred state.', { cause: value.error });
  }
  if (!('states' in value) || !Array.isArray(value.states)) {
    throw new Error('[Router] NDJSON deferred-state records must contain a states array.');
  }
  for (const state of value.states) {
    if (state !== null && (typeof state !== 'object' || Array.isArray(state))) {
      throw new Error('[Router] Each deferred state must be an object or null.');
    }
  }
  return value.states;
}

interface DeferredReadOptions {
  context: StreamingContext;
  generation: number;
  signal: AbortSignal | undefined;
  receivedState: boolean;
}

async function continueDeferredRead(
  records: NdjsonReader,
  data: StreamingPartialResponse,
  requestPath: string,
  options: DeferredReadOptions,
): Promise<void> {
  const { context, generation, signal } = options;
  let receivedState = options.receivedState;
  try {
    while (!signal?.aborted && generation === context.navGeneration) {
      const value = await records.read();
      if (signal?.aborted || generation !== context.navGeneration) return;
      if (value === undefined) {
        if (!receivedState) throw new Error('[Router] NDJSON response ended before deferred state arrived.');
        context.markCacheComplete(requestPath);
        return;
      }
      const states = readStates(value);
      persistDeferredStates(data, states);
      applyDeferredStates(states, requestPath, context);
      receivedState = true;
    }
  } finally {
    await records.cancel();
  }
}

function persistDeferredStates(
  data: StreamingPartialResponse,
  states: readonly (Record<string, unknown> | null)[],
): void {
  const chain = data.chain;
  if (!chain) return;
  for (let i = 0; i < states.length && i < chain.length; i++) {
    if (states[i] !== null) chain[i].state = states[i];
  }
}

/** Apply positional route states after commit, without overriding authored loaders. */
export function applyDeferredStates(
  states: (Record<string, unknown> | null)[],
  requestPath: string,
  context: StreamingContext,
): void {
  if (requestPath !== context.currentRequestPath) return;
  for (let i = 0; i < states.length && i < context.activeChain.length; i++) {
    const state = states[i];
    if (!hasState(state)) continue;
    const entry = context.activeChain[i];
    if (!entry.el || !entry.component) continue;
    const ctor = customElements.get(entry.component) as
      (CustomElementConstructor & { loader?: unknown }) | undefined;
    if (typeof ctor?.loader === 'function') continue;

    const component = entry.compEl ?? mountedRouteComponent(entry.el, entry.component);
    if (!component) continue;
    entry.compEl = component;
    if (isStateful(component)) component.setState(state);
    entry.state = state;
  }
}
