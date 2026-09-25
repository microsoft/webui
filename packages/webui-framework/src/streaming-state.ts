// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Shared application path for streamed state updates.
 *
 * An update never rehydrates: it drives the same reactive `setState()` a
 * browser-side caller would use. Both the immediate commit path and the queued
 * patch a late-defining island replays after activation land here, so every
 * root converges on the same final state regardless of when its definition
 * arrived. Convergence is the guarantee, not call-for-call equivalence: a root
 * that was live throughout observes each update as it commits, while a root
 * retained behind an undefined ancestor observes one collapsed patch.
 */

import { streamingErrorMessage } from './streaming-dom.js';

type StateRoot = Element & {
  setState?: (state: Record<string, unknown>) => void;
};

/**
 * Apply one shallow patch to an activated root, isolating per-root failure.
 *
 * An application component's own setter or change handler throwing degrades
 * that root alone. Rethrowing would let one app-level bug strand every later
 * boundary on the page, so the failure is reported and the caller continues.
 *
 * Returns `false` only when the target has no `setState` at all. That is a
 * framework invariant violation rather than an application error, so the
 * decision to halt belongs to the caller that owns the stream.
 */
export function applyStateUpdate(
  el: Element,
  patch: Record<string, unknown>,
): boolean {
  const root = el as StateRoot;
  if (typeof root.setState !== 'function') return false;
  try {
    root.setState(patch);
  } catch (error) {
    console.error(
      `[WebUI] streaming: state update failed for <${
        root.tagName.toLowerCase()
      }>: ${streamingErrorMessage(error)}`,
    );
  }
  return true;
}
