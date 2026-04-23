// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * NDJSON streaming — reads chunked partial responses from the server.
 * Chunk 1 contains the route chain + templates for immediate commit.
 * Chunk 2 contains deferred per-component state applied after mount.
 */

import { hasState } from './route-element.js';
import { isStateful } from './types.js';
import { registerTemplatesAndStyles, injectCssLinks } from './templates.js';
import type { PartialResponse, RouteChainEntry } from './cache.js';

/** Maximum buffered NDJSON line size before aborting the stream (256 KiB). */
export const MAX_NDJSON_BUFFER = 256 * 1024;

/** Context needed by the streaming reader to interact with router state. */
export interface StreamingContext {
  readonly navGeneration: number;
  readonly currentRequestPath: string;
  readonly activeChain: RouteChainEntry[];
  readonly nonce: string;
  readonly injectedStyles: Set<string>;
  readonly injectedCss: Set<string>;
  setDeferredReader(reader: Promise<void> | null): void;
  setDeferredGeneration(gen: number): void;
  updateInventory(inv: string): void;
  markCacheComplete(requestPath: string): void;
}

/**
 * Read a streaming NDJSON partial response.
 * Returns after Chunk 1 (chain + templates) for immediate navigation commit.
 * Spawns a background reader for Chunk 2 (deferred per-component state).
 */
export async function readStreamingPartial(
  resp: Response,
  requestPath: string,
  ctx: StreamingContext,
  signal?: AbortSignal,
  speculative?: boolean,
): Promise<(PartialResponse & { inventory?: string }) | null> {
  const reader = resp.body!.getReader();
  const decoder = new TextDecoder();
  let buffer = '';
  let chunk1: (PartialResponse & { inventory?: string }) | null = null;

  // Read until we get Chunk 1 (has 'chain' field)
  while (!chunk1) {
    const { done, value } = await reader.read();
    if (signal?.aborted) break;
    if (done) {
      // Flush remaining buffer on stream end
      buffer += decoder.decode();
      break;
    }
    buffer += decoder.decode(value, { stream: true });

    if (buffer.length > MAX_NDJSON_BUFFER) {
      console.warn('[Router] NDJSON buffer exceeded limit, aborting stream');
      reader.cancel().catch(() => {});
      return null;
    }

    const lines = buffer.split('\n');
    buffer = lines.pop()!; // keep incomplete last line

    for (const line of lines) {
      if (!line.trim()) continue;
      try {
        const parsed = JSON.parse(line);
        if (parsed.chain) {
          chunk1 = parsed;
        } else if (parsed.states && chunk1) {
          // Chunk 2 arrived in same read batch — store for post-commit application
          (chunk1 as any)._deferredStates = parsed.states;
        }
      } catch {
        // Malformed line — skip
      }
    }
  }

  // Process any final incomplete line left in buffer
  if (!chunk1 && buffer.trim()) {
    try {
      const parsed = JSON.parse(buffer);
      if (parsed.chain) chunk1 = parsed;
    } catch { /* ignore */ }
    buffer = '';
  }

  if (!chunk1 || signal?.aborted) {
    reader.cancel().catch(() => {});
    return null;
  }

  // Register templates/styles from Chunk 1
  registerTemplatesAndStyles(chunk1, ctx.nonce, ctx.injectedStyles, ctx.updateInventory);
  injectCssLinks(chunk1, ctx.injectedCss);

  // Spawn background reader for remaining chunks (Chunk 2 state)
  const gen = ctx.navGeneration;
  ctx.setDeferredGeneration(gen);
  ctx.setDeferredReader(
    continueDeferredRead(reader, decoder, buffer, requestPath, gen, ctx, signal)
      .catch((err) => {
        if (!signal?.aborted) {
          console.warn('[Router] Deferred state reader failed:', err);
        }
      }),
  );

  return chunk1;
}

/**
 * Continue reading the NDJSON stream for Chunk 2 (deferred state).
 * Runs in the background after Chunk 1 has been committed.
 */
async function continueDeferredRead(
  reader: ReadableStreamDefaultReader<Uint8Array>,
  decoder: TextDecoder,
  initialBuffer: string,
  requestPath: string,
  generation: number,
  ctx: StreamingContext,
  signal?: AbortSignal,
): Promise<void> {
  let buffer = initialBuffer;
  try {
    while (true) {
      if (signal?.aborted || generation !== ctx.navGeneration) {
        reader.cancel().catch(() => {});
        return;
      }
      const { done, value } = await reader.read();
      if (done) {
        // Flush remaining bytes from the decoder
        buffer += decoder.decode();
        break;
      }
      buffer += decoder.decode(value, { stream: true });

      if (buffer.length > MAX_NDJSON_BUFFER) {
        console.warn('[Router] NDJSON deferred buffer exceeded limit, aborting');
        reader.cancel().catch(() => {});
        return;
      }

      const lines = buffer.split('\n');
      buffer = lines.pop()!;

      for (const line of lines) {
        if (!line.trim()) continue;
        if (generation !== ctx.navGeneration) return; // Stale — stop
        try {
          const parsed = JSON.parse(line);
          if (parsed.states) {
            applyDeferredStates(parsed.states, requestPath, ctx);
          } else if (parsed.error) {
            console.warn('[Router] Streaming state error:', parsed.error);
          }
        } catch {
          // Malformed line — skip
        }
      }
    }

    // Process final incomplete line
    if (buffer.trim() && generation === ctx.navGeneration) {
      try {
        const parsed = JSON.parse(buffer);
        if (parsed.states) {
          applyDeferredStates(parsed.states, requestPath, ctx);
        } else if (parsed.error) {
          console.warn('[Router] Streaming state error:', parsed.error);
        }
      } catch { /* ignore */ }
    }
  } finally {
    // Release the stream lock and clear the deferred reference
    reader.releaseLock();
    ctx.setDeferredReader(null);
    // Mark cache entry as complete
    ctx.markCacheComplete(requestPath);
  }
}

/**
 * Apply deferred per-component states from streaming Chunk 2.
 * States array is matched 1:1 to activeChain entries by position.
 * null entries are skipped (component keeps current state).
 */
export function applyDeferredStates(
  states: (Record<string, unknown> | null)[],
  requestPath: string,
  ctx: StreamingContext,
): void {
  if (requestPath !== ctx.currentRequestPath) return; // Stale
  for (let i = 0; i < states.length && i < ctx.activeChain.length; i++) {
    const state = states[i];
    if (!hasState(state)) continue;
    const entry = ctx.activeChain[i];
    if (!entry.el || !entry.component) continue;

    // Don't override loader results
    const ctor = customElements.get(entry.component) as
      ((new () => HTMLElement) & { loader?: Function }) | undefined;
    if (ctor && typeof ctor.loader === 'function') continue;

    const compEl = entry.compEl ?? entry.el.querySelector(entry.component);
    if (!compEl) continue;
    entry.compEl = compEl;
    if (isStateful(compEl)) {
      compEl.setState(state);
    }
    // Update the chain entry's state for cache consistency
    entry.state = state;
  }
}
