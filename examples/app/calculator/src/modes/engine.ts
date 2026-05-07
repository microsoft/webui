// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Mode engine interface and registry.
 *
 * Each calculator mode (Standard, Scientific, etc.) implements ModeEngine.
 * To add a new mode: create a file, implement the interface, register it.
 */

/** Definition for a single calculator button. */
export interface ButtonDef {
  /** Display label (e.g. "sin", "7", "+") */
  label: string;
  /** Action identifier sent on press */
  value: string;
  /** Visual/behavioral category */
  type: 'number' | 'operator' | 'function' | 'action' | 'equal';
  /** Column span (default 1) */
  span?: number;
}

/** Calculator state passed through engine processing. */
export interface CalcState {
  /** Current display value */
  display: string;
  /** Full expression shown above display */
  expression: string;
  /** Memory register */
  memory: number;
  /** Whether the display should reset on next number input */
  resetOnNext: boolean;
  /** Error message, if any */
  error: string | null;
}

/** A calculator mode engine. Pure logic, no UI. */
export interface ModeEngine {
  /** Display name for the mode tab */
  readonly name: string;
  /** Number of grid columns for button layout */
  readonly columns: number;
  /** Ordered list of buttons to render */
  readonly buttons: ButtonDef[];
  /** Process a button press and return the new state. */
  processInput(input: string, state: CalcState): CalcState;
}

/** Create a fresh initial calculator state. */
export function createInitialState(): CalcState {
  return {
    display: '0',
    expression: '',
    memory: 0,
    resetOnNext: false,
    error: null,
  };
}

// --- Mode Registry ---

const registry = new Map<string, ModeEngine>();

/** Register a mode engine by key. */
export function registerMode(key: string, engine: ModeEngine): void {
  registry.set(key, engine);
}

/** Get a registered mode engine by key. Returns undefined if not found. */
export function getMode(key: string): ModeEngine | undefined {
  return registry.get(key);
}

/** Get all registered mode keys in insertion order. */
export function getModeKeys(): string[] {
  return Array.from(registry.keys());
}
