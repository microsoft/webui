// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * SSR state seeding helpers.
 *
 * During hydration the framework reads values from the SSR DOM (text content,
 * attributes, conditional visibility) and writes them back to the component's
 * `@observable` / `@attr` backing fields so that `this.count` returns the
 * server-rendered value, not the class-level default.
 *
 * These helpers are intentionally pure-ish functions that operate on a target
 * object + an observable-name set rather than reaching into `WebUIElement`
 * internals.  This keeps them independently testable and ready to be called
 * inline during the marker walk rather than in a separate second pass.
 */

/**
 * Coerce a DOM-extracted seed value to match the type of the property's
 * current (default) value.
 *
 * The DOM only stores strings, but `@observable count = 0` should seed as
 * a number, and `@observable active = false` should seed as a boolean.
 */
export function coerceSeedValue(currentValue: unknown, value: unknown): unknown {
  if (typeof currentValue === 'boolean') {
    if (typeof value === 'boolean') {
      return value;
    }
    if (typeof value === 'string') {
      return value === 'true';
    }
  }

  if (typeof currentValue === 'number') {
    if (typeof value === 'number') {
      return value;
    }
    if (typeof value === 'string') {
      const parsed = Number(value);
      if (!Number.isNaN(parsed)) {
        return parsed;
      }
    }
  }

  return value;
}

/**
 * Seed a single observable path on a target object.
 *
 * Handles both flat paths (`'count'`) and nested paths (`'item.title'`).
 * Only seeds paths whose root segment is in `observableNames`.
 * Tracks already-seeded paths in `seededPaths` to avoid double-writes.
 */
export function seedObservablePath(
  target: Record<string, unknown>,
  path: string,
  value: unknown,
  observableNames: Set<string>,
  seededPaths?: Set<string>,
): void {
  if (!path) {
    return;
  }

  const parts = path.split('.');
  const root = parts[0];
  if (!root || !observableNames.has(root)) {
    return;
  }

  if (parts.length === 1) {
    target[root] = coerceSeedValue(target[root], value);
    seededPaths?.add(path);
    return;
  }

  let obj = target[root];
  if (obj == null || Array.isArray(obj) || typeof obj !== 'object') {
    obj = {};
    target[root] = obj;
  }

  let cursor = obj as Record<string, unknown>;
  for (let index = 1; index < parts.length - 1; index += 1) {
    const key = parts[index];
    const next = cursor[key];
    if (next == null || Array.isArray(next) || typeof next !== 'object') {
      cursor[key] = {};
    }
    cursor = cursor[key] as Record<string, unknown>;
  }

  const leaf = parts[parts.length - 1];
  cursor[leaf] = coerceSeedValue(cursor[leaf], value);
  seededPaths?.add(path);
}

/**
 * Seed a condition's implied value.
 *
 * When a conditional block `@if(visible)` is shown in SSR, the framework
 * can infer `this.visible = true`.  When `@if(items.length)` is hidden,
 * the framework infers `this.items = []`.
 */
export { deriveConditionSeed } from './conditions.js';
