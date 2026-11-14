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

/** Build a scope frame for a repeat item. */
function itemScope(rep: RepeatBinding, item: unknown): ScopeFrame {
  return { name: rep.itemVar, value: item, parent: rep.scope, known: true };
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
  let created = false;

  for (let i = 0; i < reuseCount; i += 1) {
    setItemScope(instances[i], items[i]);
  }
  for (let i = reuseCount; i < items.length; i += 1) {
    const instance = host.$createBlockInstance(
      rep.blockIndex,
      itemScope(rep, items[i]),
      rep.owner,
      container,
    );
    if (instance) {
      instances[nextCount] = instance;
      nextCount += 1;
      created = true;
    }
  }
  for (let i = reuseCount; i < oldLength; i += 1) {
    host.$removeInstance(instances[i]);
  }
  instances.length = nextCount;
  const removed = oldLength > reuseCount;

  let cursor: Node | null = rep.start;
  const placed = rep.fixed ? reuseCount : 0;
  if (placed) {
    const nodes = instances[placed - 1].nodes;
    cursor = nodes[nodes.length - 1] ?? cursor;
  }
  for (let i = placed; i < instances.length; i += 1) {
    cursor = host.$insertInstanceAfter(cursor, container, instances[i]);
  }
  for (let i = 0; i < reuseCount; i += 1) {
    host.$updateInstance(instances[i]);
  }
  if (created || removed) host.$changeStructure(removed ? rep.owner : undefined);
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

function warnKeyFallback(rep: RepeatBinding, state: RepeatKeyState): void {
  if (state.warned) return;
  state.warned = true;
  const key = state.path ? `${rep.itemVar}.${state.path}` : rep.itemVar;
  console.warn(
    `[webui] repeat "${rep.collection}" produced duplicate or invalid values for child key="${key}"; using positional reconciliation`,
  );
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
  let created = false;
  let removed = false;

  for (let i = 0; i < items.length; i += 1) {
    const key = state.nextKeys[i];
    let instance = map.get(key);
    if (instance) {
      map.set(key, null);
      setItemScope(instance, items[i]);
    } else {
      instance = host.$createBlockInstance(
        rep.blockIndex,
        itemScope(rep, items[i]),
        rep.owner,
        container,
      ) ?? undefined;
      if (instance) created = true;
    }
    if (instance) {
      instances[nextCount] = instance;
      state.nextKeys[nextCount] = key;
      nextCount += 1;
    }
  }
  state.nextKeys.length = nextCount;

  for (const instance of map.values()) {
    if (instance) {
      host.$removeInstance(instance);
      removed = true;
    }
  }
  let cursor: Node | null = rep.start;
  for (let i = 0; i < nextCount; i += 1) {
    cursor = host.$insertInstanceAfter(cursor, container, instances[i]);
  }
  for (let i = 0; i < nextCount; i += 1) {
    if (map.get(state.nextKeys[i]) === null) {
      host.$updateInstance(instances[i]);
    }
  }
  map.clear();

  instances.length = nextCount;
  commitKeyIdentity(state);
  if (created || removed) host.$changeStructure(removed ? rep.owner : undefined);
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
    warnKeyFallback(rep, state);
    clearRepeatKeyState(state);
    syncPositional(host, rep, items, container);
    return;
  }

  if (state.keys.length !== rep.instances.length || order === 1) {
    syncPositional(host, rep, items, container);
    if (rep.instances.length === items.length) {
      commitKeyIdentity(state);
    } else {
      clearRepeatKeyState(state);
    }
    return;
  }

  for (let i = 0; i < state.keys.length; i += 1) {
    state.map.set(state.keys[i], rep.instances[i]);
  }
  reconcileByKey(host, rep, items, container, state);
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
