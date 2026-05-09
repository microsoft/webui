// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { RouteChainEntry, RouteStateValue, RouteStates } from './cache.js';

/**
 * Resolve the route-scoped state for one route-chain entry.
 *
 * Array `states` are index-aligned with the server-provided route chain. Object
 * `states` support index, `index:component`, component tag, and route path keys.
 */
export function stateForRouteEntry(
  states: RouteStates | undefined,
  chain: readonly RouteChainEntry[],
  index: number,
): RouteStateValue {
  if (!states) return undefined;
  const entry = chain[index];
  if (!entry) return undefined;

  if (Array.isArray(states)) {
    return states[index];
  }

  const byIndex = states[String(index)];
  if (byIndex !== undefined) return byIndex;

  if (entry.component) {
    const byScopedComponent = states[`${index}:${entry.component}`];
    if (byScopedComponent !== undefined) return byScopedComponent;

    const byComponent = states[entry.component];
    if (byComponent !== undefined) return byComponent;
  }

  if (entry.path) {
    return states[entry.path];
  }

  return undefined;
}

/**
 * Return non-null state entries that do not target the active route chain.
 */
export function outOfChainStateKeys(
  states: RouteStates | undefined,
  chain: readonly RouteChainEntry[],
): string[] {
  if (!states) return [];

  if (Array.isArray(states)) {
    if (states.length <= chain.length) return [];
    const keys: string[] = [];
    for (let i = chain.length; i < states.length; i += 1) {
      if (states[i] != null) keys.push(String(i));
    }
    return keys;
  }

  const allowed = new Set<string>();
  for (let i = 0; i < chain.length; i += 1) {
    const entry = chain[i];
    allowed.add(String(i));
    if (entry.component) {
      allowed.add(`${i}:${entry.component}`);
      allowed.add(entry.component);
    }
    if (entry.path) {
      allowed.add(entry.path);
    }
  }

  const keys = Object.keys(states);
  const unknown: string[] = [];
  for (let i = 0; i < keys.length; i += 1) {
    const key = keys[i];
    if (!allowed.has(key) && states[key] != null) {
      unknown.push(key);
    }
  }
  return unknown;
}
