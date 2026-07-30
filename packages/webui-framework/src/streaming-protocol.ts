// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { TemplateMeta } from './template.js';

const SUPPORTED_VERSION = 1;
const MAX_BOUNDARY_PAYLOAD_CHARS = 2_000_000;
const MAX_TEMPLATES_PER_BOUNDARY = 500;

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

/** Compact versioned wire record for one streamed boundary. */
export type BoundaryEnvelope = readonly [
  version: number,
  sequence: number,
  terminal: number,
  bootstrap: BoundaryBootstrap,
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

  if (!Array.isArray(parsed) || parsed.length !== 4) {
    return invalid(
      'boundary envelope must be a 4-element [version, sequence, terminal, bootstrap] array',
    );
  }

  const [version, sequence, terminal, bootstrap] = parsed;

  if (version !== SUPPORTED_VERSION) {
    return invalid(
      `unsupported boundary envelope version ${JSON.stringify(version)}`,
    );
  }
  if (!Number.isInteger(sequence) || sequence < 0) {
    return invalid('boundary sequence must be a non-negative integer');
  }
  if (terminal !== 0 && terminal !== 1) {
    return invalid('boundary terminal flag must be 0 or 1');
  }
  if (
    typeof bootstrap !== 'object' ||
    bootstrap === null ||
    Array.isArray(bootstrap)
  ) {
    return invalid('boundary bootstrap must be an object');
  }
  if (terminal === 1 && Object.keys(bootstrap).length !== 0) {
    return invalid('terminal boundary bootstrap must be empty');
  }

  const templates = (bootstrap as BoundaryBootstrap).templates;
  if (
    templates &&
    Object.keys(templates).length > MAX_TEMPLATES_PER_BOUNDARY
  ) {
    return invalid(
      `boundary declares more than ${MAX_TEMPLATES_PER_BOUNDARY} templates`,
    );
  }

  return {
    ok: true,
    envelope: [
      SUPPORTED_VERSION,
      sequence,
      terminal,
      bootstrap as BoundaryBootstrap,
    ],
  };
}
