// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Navigation cache — LRU cache with tag-based invalidation for
 * server-provided partial responses.
 */

import type { CacheConfig } from './types.js';

/** JSON partial response from the server. */
export interface PartialResponse {
  /** Top-level application state (non-streaming responses). */
  state?: Record<string, unknown>;
  /** Module CSS definitions to append before installing template closures. */
  templateStyles?: string[];
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
}

/** Maximum age (ms) for a preloaded partial before it's considered stale. */
const PRELOAD_TTL = 5_000;

export class NavigationCache {
  private cache = new Map<string, CacheEntry>();
  private tagIndex = new Map<string, Set<string>>();
  private config: Required<CacheConfig>;

  constructor(config: Required<CacheConfig>) {
    this.config = config;
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

    // Evict LRU entries if at capacity
    while (this.cache.size >= this.config.maxEntries) {
      const oldest = this.cache.keys().next().value;
      if (oldest !== undefined) {
        this.evict(oldest);
      } else {
        break;
      }
    }

    this.cache.set(requestPath, {
      data, tags, ts: Date.now(), staleTime, preload,
      complete: !preload && !streaming,
    });

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

  /** Evict a single cache entry and clean up its tag index references. */
  evict(requestPath: string): void {
    const entry = this.cache.get(requestPath);
    if (!entry) return;
    this.cache.delete(requestPath);
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
      for (const key of [...this.cache.keys()]) {
        this.evict(key);
      }
    }
  }

  /** Check if a cache entry exists for the given path (any state). */
  has(requestPath: string): boolean {
    return this.cache.has(requestPath);
  }

  /** Get the raw cache entry (for marking complete after streaming finishes). */
  getEntry(requestPath: string): CacheEntry | undefined {
    return this.cache.get(requestPath);
  }
}
