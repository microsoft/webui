// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

export const API_CHUNKS = [
  { label: 'shell', path: 'api/shell.json' },
  { label: 'hero', path: 'api/hero.json' },
  { label: 'metrics', path: 'api/metrics.json' },
  { label: 'activity', path: 'api/activity.json' },
] as const;

const ENTRY_IDS = new Set([
  'shell-panel',
  'hero-panel',
  'metrics-panel',
  'activity-panel',
]);
const MAX_DEMO_DELAY_MS = 2_000;

export type ApiChunk = (typeof API_CHUNKS)[number];

export interface ApiPayload {
  entry: string;
  state: Record<string, unknown>;
  delayMs: number;
}

export function sanitizePayload(
  payload: unknown,
  sourcePath: string,
  baseUrl: URL,
): ApiPayload {
  if (!isRecord(payload)) {
    throw new Error(`Invalid API payload from ${sourcePath}: expected object`);
  }

  const entry = readEntry(payload, sourcePath);
  const state = readState(payload, sourcePath);
  const delayMs = readDelay(payload['delayMs']);

  if (typeof state['ctaHref'] === 'string') {
    state['ctaHref'] = sanitizeHref(state['ctaHref'], sourcePath, baseUrl);
  }

  return { entry, state, delayMs };
}

function readEntry(payload: Record<string, unknown>, sourcePath: string): string {
  const entry = payload['entry'];
  if (typeof entry !== 'string' || !ENTRY_IDS.has(entry)) {
    throw new Error(`Invalid API payload from ${sourcePath}: unsupported entry`);
  }
  return entry;
}

function readState(payload: Record<string, unknown>, sourcePath: string): Record<string, unknown> {
  const state = payload['state'];
  if (!isRecord(state)) {
    throw new Error(`Invalid API payload from ${sourcePath}: missing state object`);
  }
  return { ...state };
}

function readDelay(value: unknown): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return 0;
  }
  return Math.min(Math.max(value, 0), MAX_DEMO_DELAY_MS);
}

function sanitizeHref(value: string, sourcePath: string, baseUrl: URL): string {
  let parsed: URL;
  try {
    parsed = new URL(value, baseUrl);
  } catch {
    throw new Error(`Invalid API payload from ${sourcePath}: invalid url`);
  }
  if (parsed.protocol === 'https:' || parsed.origin === baseUrl.origin) {
    return parsed.href;
  }
  throw new Error(`Invalid API payload from ${sourcePath}: unsupported link scheme`);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
