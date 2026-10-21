// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { ComponentStyles } from './types.js';
import { abortable } from './abortable.js';

/** Framework-independent resource and optional state record for a lazy body. */
export interface LazyRecord {
  type: 'webui-lazy';
  version: 1;
  fragment: string;
  owner: string;
  buildId: string;
  inventory: string;
  styleInventory: string;
  required: string[];
  templates?: Record<string, unknown>;
  templateFunctions?: Record<string, string>;
  componentStyles?: ComponentStyles;
  state?: Record<string, unknown>;
}

/** One request's resource preparation, independent of its hydration engine. */
export interface LazyReadContext {
  readonly owner: string;
  readonly fragment: string;
  readonly signal: AbortSignal;
  prepare(record: LazyRecord): Promise<void>;
  abort(reason: unknown): void;
}

/** Opt-in response decoder; JSON-only consumers do not import the NDJSON reader. */
export interface LazyTransport {
  readonly accept: string;
  read(response: Response, context: LazyReadContext): Promise<Record<string, unknown>>;
}

/** Stable address of a named region; applications should not hardcode anonymous ids. */
export function lazyFragment(owner: string, id: string): string {
  if (!owner || owner.length > 512 || !id || id.length > 128) {
    throw new Error('[WebUI] Lazy loading requires an owning template and a bounded region id.');
  }
  return `${owner}::lazy:${id}`;
}

export function lazyRecord(value: unknown, context: Pick<LazyReadContext, 'owner' | 'fragment'>): LazyRecord {
  if (!isObject(value)) throw new Error('[WebUI] Lazy responses must contain JSON objects.');
  if (value.error !== undefined) throw new Error('[WebUI] The server could not load lazy state.', { cause: value.error });
  if (value.type !== 'webui-lazy' || value.version !== 1 ||
      value.fragment !== context.fragment || value.owner !== context.owner ||
      typeof value.buildId !== 'string' || value.buildId.length === 0 ||
      !validInventory(value.inventory) || !validInventory(value.styleInventory) ||
      !Array.isArray(value.required) || !value.required.includes(context.fragment) ||
      value.required.some(id => typeof id !== 'string' || id.length === 0)) {
    throw new Error('[WebUI] Invalid or mismatched lazy response. Serve the requested region using the matching protocol.');
  }
  if (value.state !== undefined && !isObject(value.state)) {
    throw new Error('[WebUI] Lazy state must be a JSON object.');
  }
  return value as unknown as LazyRecord;
}

function validInventory(value: unknown): value is string {
  if (typeof value !== 'string' || value.length % 2 !== 0) return false;
  for (let i = 0; i < value.length; i++) {
    const code = value.charCodeAt(i);
    if (!((code >= 48 && code <= 57) || (code >= 65 && code <= 70) || (code >= 97 && code <= 102))) return false;
  }
  return true;
}

function isObject(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

export const jsonLazyTransport: LazyTransport = {
  accept: 'application/json',
  async read(response, context) {
    const record = lazyRecord(await response.json(), context);
    if (record.state === undefined) throw new Error('[WebUI] Lazy response ended without state.');
    await abortable(context.prepare(record), context.signal);
    context.signal.throwIfAborted();
    return record.state;
  },
};
