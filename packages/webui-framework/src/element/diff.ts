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

/** Build a scope frame for a repeat item. */
function itemScope(rep: RepeatBinding, item: unknown): ScopeFrame {
  return { name: rep.itemVar, value: item, parent: rep.scope, known: true };
}

/** Allocate keyed scratch state only for explicitly keyed repeats. */
export function createRepeatKeyState(path: string): RepeatKeyState {
  return {
    path,
    established: false,
    warned: false,
    keys: [],
    nextKeys: [],
    nextInstances: [],
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
  if (removed) host.$compactInstanceNodes(rep.owner);

  let cursor: Node | null = rep.start;
  for (let i = 0; i < instances.length; i += 1) {
    cursor = host.$insertInstanceAfter(cursor, container, instances[i]);
  }
  for (let i = 0; i < reuseCount; i += 1) {
    host.$updateInstance(instances[i]);
  }
  if (created || removed) host.$invalidatePathIndex();
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
    seen.set(value, undefined);
  }
  seen.clear();
  return true;
}

function keysShareOrder(current: RepeatKey[], next: RepeatKey[]): boolean {
  const sharedLength = Math.min(current.length, next.length);
  for (let i = 0; i < sharedLength; i += 1) {
    if (current[i] !== next[i]) return false;
  }
  return true;
}

function commitKeyIdentity(state: RepeatKeyState): void {
  const oldKeys = state.keys;
  state.keys = state.nextKeys;
  state.nextKeys = oldKeys;
  state.nextKeys.length = 0;
  state.established = true;
}

function clearKeyIdentity(state: RepeatKeyState): void {
  state.established = false;
  state.keys.length = 0;
  state.nextKeys.length = 0;
  state.nextInstances.length = 0;
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

function buildOldKeyMap(
  state: RepeatKeyState,
  instances: TemplateInstance[],
): boolean {
  const map = state.map;
  map.clear();
  for (let i = 0; i < state.keys.length; i += 1) {
    const key = state.keys[i];
    if (map.has(key)) {
      map.clear();
      return false;
    }
    map.set(key, instances[i]);
  }
  return true;
}

function reconcileByKey(
  host: RepeatHost,
  rep: RepeatBinding,
  items: unknown[],
  container: ParentNode & Node,
  state: RepeatKeyState,
): void {
  const oldInstances = rep.instances;
  const nextInstances = state.nextInstances;
  const map = state.map;
  let nextCount = 0;
  let created = false;
  let removed = false;
  nextInstances.length = 0;

  for (let i = 0; i < items.length; i += 1) {
    const key = state.nextKeys[i];
    let instance = map.get(key);
    if (instance) {
      map.set(key, undefined);
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
      nextInstances[nextCount] = instance;
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
  if (removed) host.$compactInstanceNodes(rep.owner);

  let cursor: Node | null = rep.start;
  for (let i = 0; i < nextCount; i += 1) {
    cursor = host.$insertInstanceAfter(cursor, container, nextInstances[i]);
  }
  for (let i = 0; i < nextCount; i += 1) {
    const key = state.nextKeys[i];
    if (map.has(key) && map.get(key) === undefined) {
      host.$updateInstance(nextInstances[i]);
    }
  }
  map.clear();

  rep.instances = nextInstances;
  state.nextInstances = oldInstances;
  state.nextInstances.length = 0;
  commitKeyIdentity(state);
  if (created || removed) host.$invalidatePathIndex();
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

  if (
    !state.established ||
    state.keys.length !== rep.instances.length ||
    keysShareOrder(state.keys, state.nextKeys)
  ) {
    syncPositional(host, rep, items, container);
    if (rep.instances.length === items.length) {
      commitKeyIdentity(state);
    } else {
      clearKeyIdentity(state);
    }
    return;
  }

  if (!buildOldKeyMap(state, rep.instances)) {
    warnKeyFallback(rep, state);
    state.established = false;
    state.keys.length = 0;
    syncPositional(host, rep, items, container);
    if (rep.instances.length === items.length) commitKeyIdentity(state);
    return;
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
    const hadInstances = rep.instances.length !== 0;
    for (let i = 0; i < rep.instances.length; i += 1) {
      host.$removeInstance(rep.instances[i]);
    }
    rep.instances.length = 0;
    if (hadInstances) {
      host.$compactInstanceNodes(rep.owner);
      host.$invalidatePathIndex();
    }
    if (rep.keyState) {
      clearKeyIdentity(rep.keyState);
      rep.keyState.established = true;
    }
    return;
  }

  if (rep.keyState) {
    syncKeyed(host, rep, items, container, rep.keyState);
  } else {
    syncPositional(host, rep, items, container);
  }
}
