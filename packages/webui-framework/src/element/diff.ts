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

/** Compute a key for an item using the cached key path, or null. */
function itemKey(item: unknown, keyPath: string | undefined): string | null {
  if (keyPath === undefined || keyPath === '') return null;
  const v = dotWalk(item, keyPath, 0);
  return v != null ? String(v) : '';
}

/** Build a scope frame for a repeat item. */
function itemScope(rep: RepeatBinding, item: unknown): ScopeFrame {
  return { name: rep.itemVar, value: item, parent: rep.scope, known: true };
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

  // Preserve SSR children until the collection root is explicitly supplied.
  // An explicit [] must still remove them, so root presence - not length -
  // distinguishes missing state from an empty collection.
  if (
    !rep.synced
    && rep.instances.length > 0
    && !host.$hasStateRoot(rep.collection, rep.scope)
  ) return;
  rep.synced = true;

  // If there are no items, just tear down everything.
  if (items.length === 0) {
    for (let i = 0; i < rep.instances.length; i += 1) {
      host.$removeInstance(rep.instances[i].instance);
    }
    rep.instances = [];
    return;
  }

  const keyPath = rep.keyPath;
  const hasKeys = keyPath !== undefined && keyPath !== '';
  const oldInstances = rep.instances;

  // ── Fast path for unkeyed (index-based) repeats ────────────────
  if (!hasKeys) {
    const oldLength = oldInstances.length;
    const reuseCount = Math.min(oldLength, items.length);
    let nextCount = reuseCount;

    // Reuse existing instances by index
    for (let i = 0; i < reuseCount; i += 1) {
      const entry = oldInstances[i];
      entry.value = items[i];
      if (entry.instance.scope) {
        entry.instance.scope.value = items[i];
        entry.instance.scope.known = true;
      }
    }

    // Create new instances for items beyond old length
    for (let i = reuseCount; i < items.length; i += 1) {
      const scope = itemScope(rep, items[i]);
      const instance = host.$createBlockInstance(rep.blockIndex, scope);
      if (instance) {
        oldInstances[nextCount] = { key: null, value: items[i], instance };
        nextCount += 1;
      }
    }

    // Remove excess old instances
    for (let i = reuseCount; i < oldLength; i += 1) {
      host.$removeInstance(oldInstances[i].instance);
    }

    oldInstances.length = nextCount;

    let cursor: Node | null = rep.start;
    for (let i = 0; i < oldInstances.length; i += 1) {
      cursor = host.$insertInstanceAfter(cursor, container, oldInstances[i].instance);
    }
    for (let i = 0; i < reuseCount; i += 1) {
      host.$updateInstance(oldInstances[i].instance);
    }
    return;
  }

  // ── Keyed diff ─────────────────────────────────────────────────

  // ── Build old-key → instance map ────────────────────────────────
  const oldByKey = new Map<string, RepeatItemInstance | undefined>();
  for (let i = 0; i < oldInstances.length; i += 1) {
    const entry = oldInstances[i];
    const k = entry.key;
    if (k != null) oldByKey.set(k, entry);
  }

  // ── Match / create ──────────────────────────────────────────────
  let nextCount = 0;
  for (let i = 0; i < items.length; i += 1) {
    const item = items[i];
    const key = itemKey(item, keyPath);
    const existing = key != null ? oldByKey.get(key) : undefined;

    if (existing) {
      oldByKey.set(key!, undefined);
      existing.value = item;
      existing.key = key;
      if (existing.instance.scope) {
        existing.instance.scope.value = item;
        existing.instance.scope.known = true;
      }
      oldInstances[nextCount] = existing;
      nextCount += 1;
    } else {
      const scope = itemScope(rep, item);
      const instance = host.$createBlockInstance(rep.blockIndex, scope);
      if (instance) {
        oldInstances[nextCount] = { key: key ?? null, value: item, instance };
        nextCount += 1;
      }
    }
  }

  // ── Remove unmatched old instances ──────────────────────────────
  for (const leftover of oldByKey.values()) {
    if (leftover) host.$removeInstance(leftover.instance);
  }
  oldInstances.length = nextCount;

  // ── Reorder DOM (forward pass) ──────────────────────────────────
  // Newly-created instances were patched while detached. Reused instances
  // update after moving so nested structural nodes stay with the item.
  let cursor: Node | null = rep.start;
  for (let i = 0; i < oldInstances.length; i += 1) {
    cursor = host.$insertInstanceAfter(cursor, container, oldInstances[i].instance);
  }
  for (let i = 0; i < oldInstances.length; i += 1) {
    const entry = oldInstances[i];
    if (entry.key != null && oldByKey.has(entry.key) && oldByKey.get(entry.key) === undefined) {
      host.$updateInstance(entry.instance);
    }
  }
}
