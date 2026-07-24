// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/** Positional and explicit-key reconciliation for `<for>` repeat blocks. */

import type {
  RepeatBinding,
  RepeatHost,
  RepeatKey,
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
  if (!collectRepeatKeys(items, state)) return;
  commitKeyIdentity(state);
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
  for (let i = 0; i < instances.length; i += 1) {
    cursor = host.$insertInstanceAfter(cursor, container, instances[i]);
  }
  for (let i = 0; i < reuseCount; i += 1) {
    host.$updateInstance(instances[i]);
  }
  if (created || removed) host.$changeStructure(removed ? rep.owner : undefined);
}

function readRepeatKey(item: unknown, path: string): unknown {
  if (path.length === 0) return item;
  return dotWalk(item, path, 0);
}

function isRepeatKey(value: unknown): value is RepeatKey {
  return (
    typeof value === 'string' ||
    (typeof value === 'number' && Number.isFinite(value))
  );
}

function collectRepeatKeys(items: unknown[], state: RepeatKeyState): boolean {
  const keys = state.nextKeys;
  const seen = state.map;
  keys.length = items.length;
  seen.clear();

  for (let i = 0; i < items.length; i += 1) {
    let value: unknown;
    try {
      value = readRepeatKey(items[i], state.path);
    } catch {
      keys.length = 0;
      seen.clear();
      return false;
    }
    if (!isRepeatKey(value) || seen.has(value)) {
      keys.length = 0;
      seen.clear();
      return false;
    }
    keys[i] = value;
    seen.set(value, null);
  }
  seen.clear();
  return true;
}

function commitKeyIdentity(state: RepeatKeyState): void {
  const oldKeys = state.keys;
  state.keys = state.nextKeys;
  state.nextKeys = oldKeys;
  state.nextKeys.length = 0;
}

function clearKeyIdentity(state: RepeatKeyState): void {
  state.keys.length = 0;
  state.nextKeys.length = 0;
  state.map.clear();
}

function warnKeyFallback(rep: RepeatBinding, state: RepeatKeyState): void {
  if (state.warned) return;
  state.warned = true;
  const key = state.path.length === 0
    ? rep.itemVar
    : `${rep.itemVar}.${state.path}`;
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
  if (!collectRepeatKeys(items, state)) {
    warnKeyFallback(rep, state);
    clearKeyIdentity(state);
    syncPositional(host, rep, items, container);
    return;
  }

  let sharedOrder = true;
  const sharedLength = Math.min(state.keys.length, state.nextKeys.length);
  for (let i = 0; sharedOrder && i < sharedLength; i += 1) {
    sharedOrder = state.keys[i] === state.nextKeys[i];
  }
  if (state.keys.length !== rep.instances.length || sharedOrder) {
    syncPositional(host, rep, items, container);
    if (rep.instances.length === items.length) {
      commitKeyIdentity(state);
    } else {
      clearKeyIdentity(state);
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

  // Locate the container once and cache it.
  const container = (rep.container
    ?? rep.start?.parentNode
    ?? rep.owner.nodes[0]?.parentNode) as (ParentNode & Node) | null;
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
    const hadInstances = rep.instances.length !== 0;
    for (let i = 0; i < rep.instances.length; i += 1) {
      host.$removeInstance(rep.instances[i]);
    }
    rep.instances.length = 0;
    if (hadInstances) host.$changeStructure(rep.owner);
    if (rep.keyState) {
      clearKeyIdentity(rep.keyState);
    }
    return;
  }

  if (rep.keyState) {
    syncKeyed(host, rep, items, container, rep.keyState);
  } else {
    syncPositional(host, rep, items, container);
  }
}
