// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { registerTemplateData } from './template.js';
import { registerComponentStyles } from './element/styles.js';
import type {
  BoundaryBootstrap,
  SpanCompletionPayload,
} from './streaming-protocol.js';

/** Register template data and merge response-scoped checkpoint metadata. */
export function applyBoundaryBootstrap(
  bootstrap: BoundaryBootstrap | SpanCompletionPayload,
): void {
  registerComponentStyles(bootstrap.componentStyles);
  if (bootstrap.templates) registerTemplateData(bootstrap.templates);

  const w = window as Window;
  if (!w.__webui) w.__webui = {};
  const keys = Object.keys(bootstrap);
  for (let i = 0; i < keys.length; i++) {
    const key = keys[i];
    // Templates are merged by registerTemplateData; boundary state remains
    // ephemeral and is handed directly to roots by the activation walk.
    if (
      key === 'templates' ||
      key === 'state' ||
      key === 'stateRef' ||
      key === 'stateDelta' ||
      key === 'componentStyles' ||
      key === 'declarationId' ||
      key === 'enclosingSpanInstanceId'
    ) continue;
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

/**
 * OR two hexadecimal inventory bitsets digit by digit.
 *
 * Both operands come from the Rust checkpoint serializer, so the digits are
 * trusted. The `typeof` guard only recovers the static type that the
 * `[key: string]: unknown` index signature on `Window['__webui']` erases.
 */
function mergeInventory(existing: unknown, delta: string | undefined): string {
  const current = typeof existing === 'string' ? existing : '';
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

/** Append the entries of one checkpoint's CSS or style delta not seen yet. */
function appendUniqueStrings(
  target: NonNullable<Window['__webui']>,
  key: 'css' | 'styles',
  delta: string[] | undefined,
): void {
  if (!delta) return;
  const existing = target[key];
  const cumulative = Array.isArray(existing) ? (existing as string[]) : [];
  for (let i = 0; i < delta.length; i++) {
    if (cumulative.indexOf(delta[i]) === -1) {
      cumulative.push(delta[i]);
    }
  }
  if (cumulative !== existing) target[key] = cumulative;
}
