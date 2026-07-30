// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { registerTemplateData } from './template.js';
import type { BoundaryBootstrap } from './streaming-protocol.js';

/** Register template data and merge response-scoped checkpoint metadata. */
export function applyBoundaryBootstrap(
  bootstrap: BoundaryBootstrap,
): void {
  if (bootstrap.templates) registerTemplateData(bootstrap.templates);

  const w = window as Window;
  if (!w.__webui) w.__webui = {};
  const keys = Object.keys(bootstrap);
  for (let i = 0; i < keys.length; i++) {
    const key = keys[i];
    // Templates are merged by registerTemplateData; boundary state remains
    // ephemeral and is handed directly to roots by the activation walk.
    if (key === 'templates' || key === 'state') continue;
    if (key === 'inventory') {
      w.__webui.inventory = mergeInventory(
        w.__webui.inventory,
        bootstrap.inventory,
      );
      continue;
    }
    if (key === 'css' || key === 'styles') {
      appendUniqueStrings(w.__webui, key, bootstrap[key]);
      continue;
    }
    w.__webui[key] = bootstrap[key];
  }
}

function mergeInventory(existing: unknown, delta: unknown): string {
  validateInventory(existing, 'existing');
  validateInventory(delta, 'boundary');
  const current = existing ?? '';
  const next = delta ?? '';
  const length = Math.max(current.length, next.length);
  let merged = '';
  for (let i = 0; i < length; i++) {
    const a = i < current.length ? parseInt(current[i], 16) : 0;
    const b = i < next.length ? parseInt(next[i], 16) : 0;
    merged += (a | b).toString(16);
  }
  return merged;
}

function validateInventory(
  value: unknown,
  source: string,
): asserts value is string | undefined {
  if (value === undefined) return;
  if (typeof value !== 'string' || value.length % 2 !== 0) {
    invalidInventory(source);
  }
  for (let i = 0; i < value.length; i++) {
    const code = value.charCodeAt(i);
    if (
      !(
        (code >= 48 && code <= 57) ||
        (code >= 65 && code <= 70) ||
        (code >= 97 && code <= 102)
      )
    ) {
      invalidInventory(source);
    }
  }
}

function invalidInventory(source: string): never {
  throw new Error(
    `${source} inventory must be an even-length hexadecimal string`,
  );
}

function appendUniqueStrings(
  target: NonNullable<Window['__webui']>,
  key: 'css' | 'styles',
  delta: unknown,
): void {
  requireStringArray(delta, `boundary ${key}`);
  const existing = target[key];
  if (existing !== undefined) requireStringArray(existing, `existing ${key}`);

  const cumulative = existing ?? [];
  for (let i = 0; i < delta.length; i++) {
    if (cumulative.indexOf(delta[i]) === -1) {
      cumulative.push(delta[i]);
    }
  }
  if (!existing) target[key] = cumulative;
}

function requireStringArray(
  value: unknown,
  source: string,
): asserts value is string[] {
  if (!Array.isArray(value)) {
    invalidStringArray(source);
  }
  for (let i = 0; i < value.length; i++) {
    if (typeof value[i] !== 'string') {
      invalidStringArray(source);
    }
  }
}

function invalidStringArray(source: string): never {
  throw new Error(`${source} must be an array of strings`);
}
