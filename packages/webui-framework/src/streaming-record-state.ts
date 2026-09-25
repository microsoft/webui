// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type {
  BoundaryBootstrap,
  SpanCompletionPayload,
} from './streaming-protocol.js';

type RangePayload = BoundaryBootstrap | SpanCompletionPayload;
type StreamedState = Record<string, unknown> | undefined;

let lastRangeSequence: number | undefined;
let lastRangeState: StreamedState;

/**
 * Resolve one range record's exact state from its complete projection or an
 * ordered reference to the immediately preceding range record.
 */
export function resolveRangeState(
  payload: RangePayload,
  sequence: number,
): StreamedState {
  const reference = payload.stateRef;
  if (reference === undefined) {
    if (payload.stateDelta !== undefined) {
      throw new Error('streaming state delta is missing its stateRef');
    }
    const state = payload.state;
    if (state !== undefined && !isStateObject(state)) {
      throw new Error('streaming range state must be an object');
    }
    lastRangeSequence = sequence;
    lastRangeState = cloneState(state);
    return state;
  }

  if (
    !Number.isSafeInteger(reference) ||
    reference < 0 ||
    reference >= sequence
  ) {
    throw new Error(`invalid streaming state reference ${String(reference)}`);
  }
  if (payload.state !== undefined) {
    throw new Error('referenced streaming state must not include complete state');
  }
  if (lastRangeSequence === undefined || reference !== lastRangeSequence) {
    throw new Error(
      `streaming state reference ${reference} does not match prior range record ${
        lastRangeSequence === undefined ? 'none' : lastRangeSequence
      }`,
    );
  }

  const delta = payload.stateDelta;
  if (delta !== undefined && !isStateObject(delta)) {
    throw new Error('streaming state delta must be an object');
  }
  const state = mergeStateDelta(lastRangeState, delta);
  lastRangeSequence = sequence;
  lastRangeState = state;
  return cloneState(state);
}

export function clearRangeState(): void {
  lastRangeSequence = undefined;
  lastRangeState = undefined;
}

function isStateObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function mergeStateDelta(
  base: StreamedState,
  delta: Record<string, unknown> | undefined,
): StreamedState {
  if (delta === undefined || Object.keys(delta).length === 0) return base;
  if (base === undefined) {
    throw new Error('streaming state delta references missing base state');
  }
  return { ...base, ...delta };
}

function cloneState(state: StreamedState): StreamedState {
  return state === undefined ? undefined : structuredClone(state);
}
