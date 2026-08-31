// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { TemplateMeta } from './template.js';
import type { ComponentStyles } from './element/styles.js';

/** Boundary-local data carried by one streamed hydration checkpoint. */
export interface BoundaryBootstrap {
  /** Compiler declaration that produced this runtime boundary occurrence. */
  declarationId: number;
  /**
   * Nearest unfinished component span enclosing this boundary occurrence.
   *
   * When present, only roots carrying the same compiler-owned enclosing-span
   * attribute may bypass that one ancestor's hydration barrier.
   */
  enclosingSpanInstanceId?: number;
  state?: Record<string, unknown>;
  /** Exact prior range-record sequence whose resolved state is the delta base. */
  stateRef?: number;
  /** Top-level additions or replacements over `stateRef`. */
  stateDelta?: Record<string, unknown>;
  templates?: Record<string, TemplateMeta>;
  inventory?: string;
  nonce?: string;
  chain?: unknown[];
  css?: string[];
  styles?: string[];
  componentStyles: ComponentStyles;
  [key: string]: unknown;
}

/** Hydrate a checkpoint once and release its roots after activation. */
export const RECORD_KIND_FINAL_CHECKPOINT = 0;
/** Hydrate a checkpoint and retain its roots for server-driven state. */
export const RECORD_KIND_UPDATABLE_CHECKPOINT = 1;
/** Apply projected state to one previously committed updatable checkpoint. */
export const RECORD_KIND_STATE_UPDATE = 2;
/** Finish one component span and activate its previously unfinished host. */
export const RECORD_KIND_SPAN_COMPLETION = 3;
/** Close the response and release all response-scoped references. */
export const RECORD_KIND_TERMINAL = 4;

export type BoundaryRecordKind =
  | typeof RECORD_KIND_FINAL_CHECKPOINT
  | typeof RECORD_KIND_UPDATABLE_CHECKPOINT
  | typeof RECORD_KIND_STATE_UPDATE
  | typeof RECORD_KIND_SPAN_COMPLETION
  | typeof RECORD_KIND_TERMINAL;

/**
 * Frozen state and metadata emitted when an unfinished component host closes.
 *
 * It deliberately reuses the bootstrap fields that register template and asset
 * deltas. Unlike a boundary checkpoint it has no boundary declaration identity:
 * the record target is a runtime SpanInstanceId in a separate kind namespace.
 */
export interface SpanCompletionPayload {
  state?: Record<string, unknown>;
  /** Exact prior range-record sequence whose resolved state is the delta base. */
  stateRef?: number;
  /** Top-level additions or replacements over `stateRef`. */
  stateDelta?: Record<string, unknown>;
  templates?: Record<string, TemplateMeta>;
  inventory?: string;
  nonce?: string;
  chain?: unknown[];
  css?: string[];
  styles?: string[];
  [key: string]: unknown;
}

export type BoundaryRecordPayload =
  | BoundaryBootstrap
  | SpanCompletionPayload
  | Record<string, unknown>;

/** Compact wire record for one streamed response operation. */
export type BoundaryEnvelope = readonly [
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
 * Parse one boundary envelope written by the Rust checkpoint serializer.
 *
 * Only two failure modes are real, so only two are checked. A response cut off
 * mid-record leaves a proper prefix of a JSON array, and every proper prefix of
 * a JSON array is invalid JSON, which makes `JSON.parse` a complete truncation
 * detector. A complete record must then have the one supported tuple shape.
 *
 * Past those checks the tuple was written by our own serializer and is not
 * re-validated. Sequence, kind, and target ordering are document state and are
 * checked by the coordinator, which fails the stream closed on a mismatch.
 */
export function parseBoundaryEnvelope(text: string): ParseBoundaryEnvelopeResult {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return invalid('boundary payload is not valid JSON');
  }

  if (!Array.isArray(parsed) || parsed.length !== 4) {
    return invalid(
      'boundary envelope must be a 4-element [recordSequence, kind, target, payload] array',
    );
  }

  return {
    ok: true,
    envelope: parsed as unknown as BoundaryEnvelope,
  };
}
