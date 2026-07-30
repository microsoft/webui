// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { TemplateMeta } from './template.js';

const SUPPORTED_VERSION = 1;
const MAX_BOUNDARY_PAYLOAD_CHARS = 2_000_000;
const MAX_TEMPLATES_PER_BOUNDARY = 500;
const MAX_STATE_UPDATE_KEYS = 10_000;

/** Boundary-local data carried by one streamed hydration checkpoint. */
export interface BoundaryBootstrap {
  state?: Record<string, unknown>;
  templates?: Record<string, TemplateMeta>;
  inventory?: string;
  nonce?: string;
  chain?: unknown[];
  css?: string[];
  styles?: string[];
  [key: string]: unknown;
}

/** Hydrate a checkpoint once and release its roots after activation. */
export const RECORD_KIND_FINAL_CHECKPOINT = 0;
/** Hydrate a checkpoint and retain its roots for server-driven state. */
export const RECORD_KIND_UPDATABLE_CHECKPOINT = 1;
/** Apply projected state to one previously committed updatable checkpoint. */
export const RECORD_KIND_STATE_UPDATE = 2;
/** Close the response and release all response-scoped references. */
export const RECORD_KIND_TERMINAL = 3;

export type BoundaryRecordKind =
  | typeof RECORD_KIND_FINAL_CHECKPOINT
  | typeof RECORD_KIND_UPDATABLE_CHECKPOINT
  | typeof RECORD_KIND_STATE_UPDATE
  | typeof RECORD_KIND_TERMINAL;

export type BoundaryRecordPayload =
  | BoundaryBootstrap
  | Record<string, unknown>;

/** Compact versioned wire record for one streamed response operation. */
export type BoundaryEnvelope = readonly [
  version: number,
  recordSequence: number,
  kind: BoundaryRecordKind,
  target: number,
  payload: BoundaryRecordPayload,
];

export type ParseBoundaryEnvelopeResult =
  | { readonly ok: true; readonly envelope: BoundaryEnvelope }
  | { readonly ok: false; readonly reason: string };

function invalid(reason: string): ParseBoundaryEnvelopeResult {
  return { ok: false, reason };
}

/**
 * Parse and structurally validate one boundary envelope.
 *
 * Sequence ordering is document state and is checked by the coordinator.
 */
export function parseBoundaryEnvelope(text: string): ParseBoundaryEnvelopeResult {
  if (text.length > MAX_BOUNDARY_PAYLOAD_CHARS) {
    return invalid(
      `boundary payload exceeds ${MAX_BOUNDARY_PAYLOAD_CHARS} characters`,
    );
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return invalid('boundary payload is not valid JSON');
  }

  if (!Array.isArray(parsed) || parsed.length !== 5) {
    return invalid(
      'boundary envelope must be a 5-element [version, recordSequence, kind, target, payload] array',
    );
  }

  const [version, recordSequence, kind, target, payload] = parsed;

  if (version !== SUPPORTED_VERSION) {
    return invalid(
      `unsupported boundary envelope version ${JSON.stringify(version)}`,
    );
  }
  if (!Number.isInteger(recordSequence) || recordSequence < 0) {
    return invalid('record sequence must be a non-negative integer');
  }
  const kindNumber = kind as number;
  if (
    !Number.isInteger(kindNumber) ||
    kindNumber < RECORD_KIND_FINAL_CHECKPOINT ||
    kindNumber > RECORD_KIND_TERMINAL
  ) {
    return invalid('boundary record kind must be 0, 1, 2, or 3');
  }
  if (!Number.isInteger(target) || target < 0) {
    return invalid('boundary target must be a non-negative integer');
  }
  if (
    typeof payload !== 'object' ||
    payload === null ||
    Array.isArray(payload)
  ) {
    return invalid('boundary record payload must be an object');
  }
  if (
    kind === RECORD_KIND_TERMINAL &&
    (target !== 0 || Object.keys(payload).length !== 0)
  ) {
    return invalid('terminal boundary record must target 0 with an empty payload');
  }

  const templates =
    kind === RECORD_KIND_FINAL_CHECKPOINT ||
    kind === RECORD_KIND_UPDATABLE_CHECKPOINT
      ? (payload as BoundaryBootstrap).templates
      : undefined;
  if (
    templates &&
    Object.keys(templates).length > MAX_TEMPLATES_PER_BOUNDARY
  ) {
    return invalid(
      `boundary declares more than ${MAX_TEMPLATES_PER_BOUNDARY} templates`,
    );
  }
  if (
    kind === RECORD_KIND_STATE_UPDATE &&
    Object.keys(payload).length > MAX_STATE_UPDATE_KEYS
  ) {
    return invalid(
      `state update declares more than ${MAX_STATE_UPDATE_KEYS} keys`,
    );
  }

  return {
    ok: true,
    envelope: parsed as unknown as BoundaryEnvelope,
  };
}
