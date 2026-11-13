// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Navigation cache — LRU cache with tag-based invalidation for
 * server-provided partial responses.
 */

import type { CacheConfig, ComponentStyles } from './types.js';
import type { TemplateResourceResponse } from './templates.js';

/** JSON partial response from the server. */
export interface PartialResponse extends TemplateResourceResponse {
  /** Top-level application state (non-streaming responses). */
  state?: Record<string, unknown>;
  /** Versioned component stylesheet definitions and ordered closures. */
  componentStyles: ComponentStyles;
  /** JSON-safe template metadata keyed by component tag. */
  templates: Record<string, unknown>;
  /** JavaScript condition closure arrays keyed by component tag. */
  templateFunctions?: Record<string, string>;
  path: string;
  chain?: RouteChainEntry[];
  /** CSS stylesheet URLs to inject into `<head>` for this route's components. */
  css?: string[];
  /** Resolved cache tags for this route chain (union of all levels). */
  cacheTags?: string[];
  /** Server-provided cache control overrides. */
  cacheControl?: { staleTime?: number };
}

/** An entry in the matched route chain, one per nesting level. */
export interface RouteChainEntry {
  /** Component tag name for this route level. */
  component: string;
  /** Route path pattern as declared in the template. */
  path: string;
  /** Bound route parameters at this level. */
  params: Record<string, string>;
  /** Whether this route requires an exact match. */
  exact?: boolean;
  /** DOM element, populated during mount or SSR chain build. */
  el?: HTMLElement;
  /** Cached component element inside the route (set after mount). */
  compEl?: Element;
  /** Comma-separated allowlist of query params forwarded as attributes. */
  allowedQuery?: string;
  /** When true, the component is preserved across navigations instead of re-created. */
  keepAlive?: boolean;
  /** Component tag for pending/loading UI. */
  pendingComponent?: string;
  /** Component tag for error boundary UI. */
  errorComponent?: string;
  /** Invalidation tags from the build-time proto (already resolved with params). */
  invalidates?: string[];
  /**
   * Per-component state from the server.
   * - `undefined` → skip setState (preserve component's current state)
   * - `null` → skip setState (preserve component's current state)
   * - `{...}` → call setState with this data
   */
  state?: Record<string, unknown> | null;
}

/** A single entry in the navigation cache. */
export interface CacheEntry {
  /** The full partial response data. */
  data: PartialResponse & { inventory?: string };
  /** Cache tags associated with this entry (from server response). */
  tags: string[];
  /** Timestamp when this entry was stored. */
  ts: number;
  /** Server-provided stale time override (ms), or undefined to use config default. */
  staleTime?: number;
  /** True if this entry came from a speculative preload fetch. */
  preload?: boolean;
  /** Whether both streaming chunks have been received. */
  complete: boolean;
  /** Estimated decoded response size, including its request-path string. */
  bytes: number;
  /** Per-route state accounting exists only while a streamed response is incomplete. */
  stateBytes?: number[];
}

/** Maximum age (ms) for a preloaded partial before it's considered stale. */
const PRELOAD_TTL = 5_000;
const DEFAULT_MAX_BYTES = 4 * 1024 * 1024;
// UTF-16 "state" property name and its reference slot, separate from the value.
const STATE_PROPERTY_BYTES = 18;

/** Select and configure a navigation response cache. Resource inventories remain separate. */
export function navigationCache(config: CacheConfig = {}): NavigationCache {
  const resolved = { staleTime: 0, gcTime: 300_000, maxEntries: 50, ...config };
  if (!Number.isSafeInteger(resolved.maxEntries) || resolved.maxEntries <= 0 ||
      !Number.isFinite(resolved.staleTime) || resolved.staleTime < 0 ||
      !Number.isFinite(resolved.gcTime) || resolved.gcTime < 0) {
    throw new RangeError('[WebUI] Cache maxEntries must be a positive integer and lifetimes must be finite and nonnegative.');
  }
  return new NavigationCache(resolved);
}

export class NavigationCache {
  private cache = new Map<string, CacheEntry>();
  private tagIndex = new Map<string, Set<string>>();
  private config: Required<CacheConfig>;
  private bytes = 0;

  constructor(config: Required<Omit<CacheConfig, 'maxBytes'>> & Pick<CacheConfig, 'maxBytes'>) {
    const maxBytes = config.maxBytes ?? DEFAULT_MAX_BYTES;
    if (!Number.isSafeInteger(maxBytes) || maxBytes < 0) {
      throw new RangeError('[WebUI] Cache maxBytes must be a nonnegative safe integer.');
    }
    this.config = { ...config, maxBytes };
  }

  /** Look up a cache entry. Returns null if missing, stale, or incomplete. */
  lookup(requestPath: string): (PartialResponse & { inventory?: string }) | null {
    const entry = this.cache.get(requestPath);
    if (!entry || !entry.complete) return null;

    const age = Date.now() - entry.ts;
    const staleTime = entry.staleTime ?? this.config.staleTime;

    // Preload entries get a minimum 5s freshness window; normal entries use staleTime as-is
    const effectiveStaleTime = entry.preload ? Math.max(staleTime, PRELOAD_TTL) : staleTime;
    if (age > effectiveStaleTime) {
      return null; // Stale — let handleNavigation refetch
    }

    // LRU: delete + reinsert to move to end (most recently used)
    this.cache.delete(requestPath);
    this.cache.set(requestPath, entry);
    return entry.data;
  }

  /** Store a partial response in the cache with its tags. */
  store(
    requestPath: string,
    data: PartialResponse & { inventory?: string },
    preload?: boolean,
    streaming?: boolean,
  ): void {
    const tags = data.cacheTags ?? [];
    const staleTime = data.cacheControl?.staleTime;

    // Clean up old tag-index references before overwriting
    this.evict(requestPath);

    if (!streaming && !preload && (staleTime ?? this.config.staleTime) === 0) return;
    if (streaming && data.chain && data.chain.length > this.config.maxBytes / (STATE_PROPERTY_BYTES + 8)) return;
    const stateBytes = streaming && data.chain ? new Array<number>(data.chain.length) : undefined;
    const bytes = this.measure(requestPath, data, stateBytes);
    if (bytes > this.config.maxBytes) return;

    // Evict LRU entries if at capacity
    while (this.cache.size >= this.config.maxEntries || this.bytes + bytes > this.config.maxBytes) {
      const oldest = this.cache.keys().next().value;
      if (oldest !== undefined) {
        this.evict(oldest);
      } else {
        break;
      }
    }

    this.cache.set(requestPath, {
      data, tags, ts: Date.now(), staleTime, preload,
      complete: !streaming, bytes, stateBytes,
    });
    this.bytes += bytes;

    // Build reverse tag index
    for (const tag of tags) {
      let paths = this.tagIndex.get(tag);
      if (!paths) {
        paths = new Set();
        this.tagIndex.set(tag, paths);
      }
      paths.add(requestPath);
    }
  }

  /** Settle only the stream that owns this entry, releasing ordinary zero-freshness data. */
  settleStream(requestPath: string, data: PartialResponse, completed: boolean, accounted = false): void {
    if (!accounted) this.refreshStream(requestPath, data);
    const entry = this.cache.get(requestPath);
    if (!entry || entry.data !== data) return;
    if (!entry.preload && (entry.staleTime ?? this.config.staleTime) === 0) {
      this.evict(requestPath);
    } else if (completed) {
      entry.complete = true;
      entry.stateBytes = undefined;
    }
  }

  /** Reaccount a growing stream only while it still owns the cached entry. */
  refreshStream(
    requestPath: string, data: PartialResponse,
    states?: readonly (Record<string, unknown> | null)[],
  ): void {
    const entry = this.cache.get(requestPath);
    if (!entry || entry.data !== data) return;
    const sizes = entry.stateBytes;
    let bytes = entry.bytes;
    if (states && sizes) {
      for (let i = 0; i < states.length && i < sizes.length; i++) {
        if (states[i] === null) continue;
        const next = STATE_PROPERTY_BYTES + this.measureValue(states[i]);
        bytes += next - sizes[i];
        sizes[i] = next;
      }
    } else bytes = this.measure(requestPath, data, sizes);
    if (bytes > this.config.maxBytes) {
      this.evict(requestPath);
      return;
    }
    this.bytes += bytes - entry.bytes;
    entry.bytes = bytes;
    while (this.bytes > this.config.maxBytes) {
      const oldest = this.cache.keys().next().value;
      if (oldest === undefined) break;
      this.evict(oldest);
    }
  }

  private measure(path: string, data: PartialResponse, sizes?: number[]): number {
    const chain = sizes ? data.chain : undefined;
    let bytes = path.length * 2 + this.measureValue(data, chain);
    if (chain && sizes) {
      for (let i = 0; i < chain.length && bytes <= this.config.maxBytes; i++) {
        sizes[i] = STATE_PROPERTY_BYTES + this.measureValue(chain[i].state);
        bytes += sizes[i];
      }
    }
    return bytes;
  }

  private measureValue(data: unknown, stateOwners?: RouteChainEntry[]): number {
    let bytes = 8;
    const stack: unknown[] = [data];
    const seen = new Set<object>();
    while (stack.length && bytes <= this.config.maxBytes) {
      const value = stack.pop();
      if (typeof value === 'string') {
        bytes += value.length * 2;
      } else if (value !== null && typeof value === 'object' && !seen.has(value)) {
        seen.add(value);
        bytes += 24;
        const keys = Object.keys(value);
        const array = Array.isArray(value);
        for (const key of keys) {
          if (key === 'state' && stateOwners?.includes(value as RouteChainEntry)) continue;
          bytes += 8 + (array ? 0 : key.length * 2);
          if (bytes > this.config.maxBytes) return bytes;
          const child = (value as Record<string, unknown>)[key];
          if (typeof child === 'string') bytes += child.length * 2;
          else if (child !== null && typeof child === 'object') stack.push(child);
        }
      }
    }
    return bytes;
  }

  /** Evict a single cache entry and clean up its tag index references. */
  evict(requestPath: string): void {
    const entry = this.cache.get(requestPath);
    if (!entry) return;
    this.cache.delete(requestPath);
    this.bytes -= entry.bytes;
    for (const tag of entry.tags) {
      const paths = this.tagIndex.get(tag);
      if (paths) {
        paths.delete(requestPath);
        if (paths.size === 0) this.tagIndex.delete(tag);
      }
    }
  }

  /** Run GC: evict entries older than gcTime. */
  gc(): void {
    const now = Date.now();
    const gcTime = this.config.gcTime;
    for (const [path, entry] of this.cache) {
      if (now - entry.ts > gcTime) {
        this.evict(path);
      }
    }
  }

  /** Clear all entries. */
  clear(): void {
    this.cache.clear();
    this.tagIndex.clear();
    this.bytes = 0;
  }

  /** Invalidate all cache entries whose tags overlap with the given tags. */
  invalidateTags(tags: string[]): void {
    if (tags.length === 0) return;
    const pathsToEvict = new Set<string>();
    for (const tag of tags) {
      const paths = this.tagIndex.get(tag);
      if (paths) {
        for (const path of paths) pathsToEvict.add(path);
      }
    }
    for (const path of pathsToEvict) {
      this.evict(path);
    }
  }

  /** Invalidate cache entries by path, or all entries if no path is given. */
  invalidate(path?: string): void {
    if (path) {
      this.evict(path);
    } else {
      this.clear();
    }
  }

  /** Check if a cache entry exists for the given path (any state). */
  has(requestPath: string): boolean {
    return this.cache.has(requestPath);
  }

  /** Get the raw cache entry. */
  getEntry(requestPath: string): CacheEntry | undefined {
    return this.cache.get(requestPath);
  }
}
