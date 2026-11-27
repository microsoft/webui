// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/** Positional and explicit-key reconciliation for `<for>` repeat blocks. */

import type {
  RepeatBinding,
  RepeatHost,
  RepeatKeyState,
  ScopeFrame,
  TemplateInstance,
} from './types.js';

// ── Helpers ─────────────────────────────────────────────────────────

/** Resolve a dotted path from a start offset without allocating. */
export function dotWalk(cursor: unknown, path: string, from: number): unknown {
  while (from <= path.length) {
    const dot = path.indexOf('.', from);
    const key = path.slice(from, dot < 0 ? undefined : dot);
    if (cursor == null || typeof cursor !== 'object') {
      return dot < 0 && key === 'length' && typeof cursor === 'string'
        ? utf8Length(cursor)
        : undefined;
    }
    cursor = (cursor as Record<string, unknown>)[key];
    if (dot < 0) break;
    from = dot + 1;
  }
  return cursor;
}

function utf8Length(value: string): number {
  let bytes = value.length;
  let point: number | undefined;
  for (let i = 0; (point = value.codePointAt(i)) !== undefined; i++) {
    if (point >= 0x80) bytes += point < 0x800 ? 1 : 2;
    if (point >= 0x10000) i++;
  }
  return bytes;
}

function createRepeatItem(
  host: RepeatHost,
  rep: RepeatBinding,
  item: unknown,
  container: ParentNode & Node,
): TemplateInstance | null {
  const scope: ScopeFrame = { name: rep.itemVar, value: item, parent: rep.scope, known: true };
  return host.$createBlockInstance(rep.blockIndex, scope, rep.owner, container);
}

/** Allocate keyed scratch state only for explicitly keyed repeats. */
export function createRepeatKeyState(path: string): RepeatKeyState {
  return {
    path,
    warned: false,
    keys: [],
    nextKeys: [],
    map: new Map(),
  };
}

/** Establish keyed identity from the bootstrap collection that produced SSR. */
export function seedHydratedRepeatKeys(
  rep: RepeatBinding,
  items: unknown[],
): void {
  const state = rep.keyState;
  if (!state || items.length !== rep.instances.length) return;
  if (collectRepeatKeys(items, state)) commitKeyIdentity(state);
  else clearRepeatKeyState(state);
}

function setItemScope(instance: TemplateInstance, item: unknown): void {
  if (!instance.scope) return;
  instance.scope.value = item;
  instance.scope.known = true;
}

function placeInstances(
  host: RepeatHost,
  rep: RepeatBinding,
  container: ParentNode & Node,
  start: number,
  end: number,
): void {
  const instances = rep.instances;
  let cursor: Node | null = rep.start;
  if (start) {
    const nodes = instances[start - 1].nodes;
    cursor = nodes[nodes.length - 1] ?? cursor;
  }
  for (let i = start; i < end; i++) {
    cursor = host.$insertInstanceAfter(cursor, container, instances[i]);
  }
}

function syncPositional(
  host: RepeatHost,
  rep: RepeatBinding,
  items: unknown[],
  container: ParentNode & Node,
): void {
  const instances = rep.instances;
  const oldLength = instances.length;
  const reuseCount = Math.min(oldLength, items.length);
  let nextCount = reuseCount;

  for (let i = 0; i < reuseCount; i += 1) {
    setItemScope(instances[i], items[i]);
  }
  for (let i = reuseCount; i < items.length; i += 1) {
    const instance = createRepeatItem(host, rep, items[i], container);
    if (instance) {
      instances[nextCount++] = instance;
    }
  }
  for (let i = reuseCount; i < oldLength; i += 1) {
    host.$removeInstance(instances[i]);
  }
  instances.length = nextCount;
  const removed = oldLength > reuseCount;

  placeInstances(host, rep, container, rep.fixed ? reuseCount : 0, nextCount);
  for (let i = 0; i < reuseCount; i += 1) {
    host.$updateInstance(instances[i]);
  }
  if (nextCount > reuseCount || removed) host.$changeStructure(removed ? rep.owner : undefined);
}

function collectRepeatKeys(items: unknown[], state: RepeatKeyState): 0 | 1 | 2 {
  const keys = state.nextKeys;
  const seen = state.map;
  const path = state.path;
  keys.length = items.length;
  seen.clear();
  let sharedOrder = true;

  try {
    for (let i = 0; i < items.length; i += 1) {
      const value = path ? dotWalk(items[i], path, 0) : items[i];
      if (typeof value !== 'string' && (typeof value !== 'number' || !Number.isFinite(value))) {
        return 0;
      }
      keys[i] = value;
      if (i < state.keys.length && value !== state.keys[i]) sharedOrder = false;
    }
  } catch {
    return 0;
  }
  // An unchanged prefix inherits its uniqueness proof, but not its values.
  if (!sharedOrder || items.length > state.keys.length) {
    for (const key of keys) seen.set(key, null);
    const unique = seen.size === keys.length;
    seen.clear();
    if (!unique) return 0;
  }
  return sharedOrder ? 1 : 2;
}

function commitKeyIdentity(state: RepeatKeyState): void {
  const oldKeys = state.keys;
  state.keys = state.nextKeys;
  state.nextKeys = oldKeys;
  oldKeys.length = 0;
}

/** Release retained keys and scratch indexes when a repeat loses its identity. */
export function clearRepeatKeyState(state: RepeatKeyState): void {
  state.keys.length = 0;
  state.nextKeys.length = 0;
  state.map.clear();
}

function reconcileByKey(
  host: RepeatHost,
  rep: RepeatBinding,
  items: unknown[],
  container: ParentNode & Node,
  state: RepeatKeyState,
): void {
  const instances = rep.instances;
  const map = state.map;
  let nextCount = 0;
  let reused = 0;

  for (let i = 0; i < items.length; i += 1) {
    const key = state.nextKeys[i];
    let instance = map.get(key);
    if (instance) {
      map.set(key, null);
      setItemScope(instance, items[i]);
      reused++;
    } else {
      instance = createRepeatItem(host, rep, items[i], container);
    }
    if (instance) {
      instances[nextCount] = instance;
      state.nextKeys[nextCount] = key;
      nextCount += 1;
    }
  }
  state.nextKeys.length = nextCount;

  const removed = reused < map.size;
  if (removed) {
    for (const instance of map.values()) {
      if (instance) host.$removeInstance(instance);
    }
  }
  placeInstances(host, rep, container, 0, nextCount);
  for (let i = 0; i < nextCount; i += 1) {
    if (map.has(state.nextKeys[i])) {
      host.$updateInstance(instances[i]);
    }
  }
  map.clear();

  instances.length = nextCount;
  commitKeyIdentity(state);
  if (reused < nextCount || removed) host.$changeStructure(removed ? rep.owner : undefined);
}

function syncKeyed(
  host: RepeatHost,
  rep: RepeatBinding,
  items: unknown[],
  container: ParentNode & Node,
  state: RepeatKeyState,
): void {
  const order = collectRepeatKeys(items, state);
  if (!order) {
    if (!state.warned) {
      state.warned = true;
      console.warn('[webui] Invalid repeat key; using positions:', rep.collection, state.path);
    }
    clearRepeatKeyState(state);
  } else if (order === 2 && state.keys.length === rep.instances.length) {
    for (let i = 0; i < state.keys.length; i += 1) {
      state.map.set(state.keys[i], rep.instances[i]);
    }
    reconcileByKey(host, rep, items, container, state);
    return;
  }

  syncPositional(host, rep, items, container);
  if (order) {
    if (rep.instances.length === items.length) {
      commitKeyIdentity(state);
    } else {
      clearRepeatKeyState(state);
    }
  }
}

// ── Reconciliation ──────────────────────────────────────────────────

/**
 * Reconcile a repeat binding against its current collection value.
 *
 * Called by `$updateInstance` on every reactive update.  Resolves the
 * collection path and applies either positional or explicit-key identity.
 */
export function syncRepeat(
  host: RepeatHost,
  rep: RepeatBinding,
): void {
  const resolved = host.$resolveValue(rep.collection, rep.scope);
  const items = Array.isArray(resolved) ? resolved : [];

  // Wiring and relocation own the live container; disposal clears it.
  const container = rep.container;
  if (!container) return;

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
    const hadInstances = rep.instances.length !== 0;
    for (let i = 0; i < rep.instances.length; i += 1) {
      host.$removeInstance(rep.instances[i]);
    }
    rep.instances.length = 0;
    if (hadInstances) host.$changeStructure(rep.owner);
    if (rep.keyState) {
      clearRepeatKeyState(rep.keyState);
    }
    return;
  }

  if (rep.keyState) {
    syncKeyed(host, rep, items, container, rep.keyState);
  } else {
    syncPositional(host, rep, items, container);
  }
}
