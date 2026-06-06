// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Keyed child reconciliation for `@for(item of items)` repeat blocks.
 *
 * Diff that matches old instances by key, reuses what it
 * can, creates/removes the rest, then reorders DOM nodes in one forward pass.
 */

import type {
  RepeatBinding,
  RepeatHost,
  RepeatItemInstance,
  ScopeFrame,
} from './types.js';

// ── Helpers ─────────────────────────────────────────────────────────

function asParent(node: Node | null): (ParentNode & Node) | null {
  if (!node) return null;
  return 'childNodes' in node ? (node as ParentNode & Node) : null;
}

/** Resolve a dotted path from a start offset without allocating. */
export function dotWalk(cursor: unknown, path: string, from: number): unknown {
  let start = from;
  for (let i = from; i <= path.length; i++) {
    if (i === path.length || path.charCodeAt(i) === 46 /* . */) {
      if (cursor == null || typeof cursor !== 'object') return undefined;
      cursor = (cursor as Record<string, unknown>)[path.slice(start, i)];
      start = i + 1;
    }
  }
  return cursor;
}

/**
 * Resolve a dotted path against a repeat scope variable.
 *
 * When a binding inside `@for(item of items)` references `item.title`,
 * this function looks up `title` on the current scope value.
 */
export function resolveRepeatValue(
  scopeVar: string,
  scope: unknown,
  path: string,
): unknown {
  if (path === scopeVar) return scope;
  if (path.length <= scopeVar.length || path.charCodeAt(scopeVar.length) !== 46 /* '.' */ || !path.startsWith(scopeVar)) return undefined;
  return dotWalk(scope, path, scopeVar.length + 1);
}

/** Compute a key for an item using the cached key path, or null. */
function itemKey(item: unknown, keyPath: string | undefined): string | null {
  if (keyPath === undefined || keyPath === '') return null;
  const v = dotWalk(item, keyPath, 0);
  return v != null ? String(v) : '';
}

/** Build a scope frame for a repeat item. */
function itemScope(rep: RepeatBinding, item: unknown): ScopeFrame {
  return { name: rep.itemVar, value: item, parent: rep.scope };
}

// ── Reconciliation ──────────────────────────────────────────────────

/**
 * Reconcile a repeat binding against its current collection value.
 *
 * Called by `$updateInstance` on every reactive update.  Resolves the
 * collection path, diffs old vs. new items by key, and patches the DOM.
 */
export function syncRepeat(
  host: RepeatHost,
  rep: RepeatBinding,
): void {
  const resolved = host.$resolveValue(rep.collection, rep.scope);
  const items = Array.isArray(resolved) ? resolved : [];

  // Locate the container once and cache it.
  let container = rep.container
    ?? (rep.start ? asParent(rep.start.parentNode) : null)
    ?? (rep.owner.nodes[0] ? asParent(rep.owner.nodes[0].parentNode) : null);
  if (!container) return;
  rep.container = container;

  // Before the first client-side sync, bail if the collection hasn't
  // been explicitly set but SSR children already exist.
  if (!rep.synced && items.length === 0 && rep.instances.length > 0) return;
  rep.synced = true;

  // If there are no items, just tear down everything.
  if (items.length === 0) {
    for (let i = 0; i < rep.instances.length; i += 1) {
      host.$removeInstance(rep.instances[i].instance);
    }
    rep.instances = [];
    return;
  }

  const keyPath = Object.values(rep.attrMap)[0];
  const hasKeys = keyPath !== undefined && keyPath !== '';
  const oldInstances = rep.instances;

  // ── Fast path for unkeyed (index-based) repeats ────────────────
  if (!hasKeys) {
    const next: RepeatItemInstance[] = [];
    const reuseCount = Math.min(oldInstances.length, items.length);

    // Reuse existing instances by index
    for (let i = 0; i < reuseCount; i += 1) {
      const entry = oldInstances[i];
      entry.value = items[i];
      if (entry.instance.scope) entry.instance.scope.value = items[i];
      next.push(entry);
    }

    // Create new instances for items beyond old length
    for (let i = reuseCount; i < items.length; i += 1) {
      const scope = itemScope(rep, items[i]);
      const instance = host.$createBlockInstance(rep.blockIndex, scope);
      if (instance) {
        next.push({ key: null, value: items[i], instance });
      }
    }

    // Remove excess old instances
    for (let i = reuseCount; i < oldInstances.length; i += 1) {
      host.$removeInstance(oldInstances[i].instance);
    }

    rep.instances = next;

    let cursor: Node | null = rep.start;
    for (let i = 0; i < next.length; i += 1) {
      cursor = host.$insertInstanceAfter(cursor, container, next[i].instance);
    }
    for (let i = 0; i < reuseCount; i += 1) {
      host.$updateInstance(next[i].instance);
    }
    return;
  }

  // ── Keyed diff ─────────────────────────────────────────────────

  // ── Build old-key → instance map ────────────────────────────────
  const oldByKey = new Map<string, RepeatItemInstance>();
  for (let i = 0; i < oldInstances.length; i += 1) {
    const entry = oldInstances[i];
    const k = entry.key;
    if (k != null) oldByKey.set(k, entry);
  }

  // ── Match / create ──────────────────────────────────────────────
  const next: RepeatItemInstance[] = [];
  for (let i = 0; i < items.length; i += 1) {
    const item = items[i];
    const key = itemKey(item, keyPath);
    const existing = key != null ? oldByKey.get(key) : undefined;

    if (existing) {
      oldByKey.delete(key!);
      existing.value = item;
      existing.key = key;
      if (existing.instance.scope) existing.instance.scope.value = item;
      next.push(existing);
    } else {
      const scope = itemScope(rep, item);
      const instance = host.$createBlockInstance(rep.blockIndex, scope);
      if (instance) {
        next.push({ key: key ?? null, value: item, instance });
      }
    }
  }

  // ── Remove unmatched old instances ──────────────────────────────
  for (const leftover of oldByKey.values()) {
    host.$removeInstance(leftover.instance);
  }
  rep.instances = next;

  // ── Reorder DOM (forward pass) ──────────────────────────────────
  // Newly-created instances were patched while detached. Reused instances
  // update after moving so nested structural nodes stay with the item.
  let cursor: Node | null = rep.start;
  for (let i = 0; i < next.length; i += 1) {
    cursor = host.$insertInstanceAfter(cursor, container, next[i].instance);
  }
  for (let i = 0; i < oldInstances.length; i += 1) {
    const entry = oldInstances[i];
    const k = entry.key;
    if (k != null && !oldByKey.has(k)) host.$updateInstance(entry.instance);
  }
}
