// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { registerTemplateData } from './template-registry.js';
import { registerComponentStyles } from './element/style-catalog.js';
import type {
  BoundaryBootstrap,
  SpanCompletionPayload,
} from './streaming-protocol.js';

/** Register template data and merge response-scoped checkpoint metadata. */
export function applyBoundaryBootstrap(
  bootstrap: BoundaryBootstrap | SpanCompletionPayload,
): void {
  const buildId = bootstrap.buildId;
  const expected = window.__webui?.buildId;
  if (buildId !== undefined && (
    typeof buildId !== 'string' || buildId.length === 0 ||
    (expected !== undefined && expected !== buildId)
  )) {
    throw new Error('[WebUI] Resources use a different document build. Reload the document.');
  }
  registerComponentStyles(bootstrap.componentStyles);
  if (bootstrap.templates) registerTemplateData(bootstrap.templates);

  const runtime = window.__webui ??= {};
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
    if (key === 'inventory' || key === 'styleInventory') {
      runtime[key] = mergeInventory(runtime[key], bootstrap[key]);
      continue;
    }
    if (key === 'css' || key === 'styles') {
      appendUniqueStrings(runtime, key, bootstrap[key]);
      continue;
    }
    runtime[key] = bootstrap[key];
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
