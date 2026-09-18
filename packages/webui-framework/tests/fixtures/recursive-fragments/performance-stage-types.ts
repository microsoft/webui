// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/** Harness-only snapshots must never retain input objects or DOM references. */
export interface FragmentCheckpoint {
  phase: 'input-ready' | 'mounted';
  lane: 'client' | 'ssr';
  evidence?: Readonly<Record<string, string | number | boolean | null>>;
  timings?: Readonly<Record<string, number>>;
}

declare global {
  interface Window {
    /** The harness resolves each checkpoint after its out-of-band measurements. */
    __webuiFragmentCheckpoint?: (checkpoint: FragmentCheckpoint) => Promise<void>;
  }
}
