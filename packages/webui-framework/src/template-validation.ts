// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type {
  CompiledConditionFn,
  TemplateMeta,
} from './template-types.js';

interface ConditionOptions {
  allowInlineFunctions: boolean;
  functions: readonly CompiledConditionFn[];
  pending: PendingCondition[];
}

interface PendingCondition {
  condition: unknown[];
  fn: CompiledConditionFn;
}

/** Validate one compiled template and resolve its condition closure indexes. */
export function validateAndNormalizeTemplate(
  name: string,
  value: unknown,
  functions: readonly CompiledConditionFn[],
  allowInlineFunctions: boolean,
): TemplateMeta {
  const root = templateBlock(name, value, 'metadata');
  const blocks = optionalArray(name, root.b, 'nested template blocks');
  const pending: PendingCondition[] = [];
  const options = { allowInlineFunctions, functions, pending };

  validateBlock(name, root, blocks.length, options);
  for (let i = 0; i < blocks.length; i++) {
    validateBlock(
      name,
      templateBlock(name, blocks[i], `nested template block ${i}`),
      blocks.length,
      options,
    );
  }
  validateRootFields(name, root);
  for (let i = 0; i < pending.length; i++) {
    pending[i].condition[0] = pending[i].fn;
  }
  return root as unknown as TemplateMeta;
}

function validateBlock(
  name: string,
  block: Record<string, unknown>,
  blockCount: number,
  options: ConditionOptions,
): void {
  validateTextRuns(name, block.tx);
  validateAttributes(name, block.a, options);
  validateAttributeGroups(name, block.ag);
  validateConditionals(name, block.c, blockCount, options);
  validateRepeats(name, block.r, blockCount);
  validateEventGroups(name, block.eg);
}

function validateTextRuns(name: string, value: unknown): void {
  const entries = optionalArray(name, value, 'text runs');
  for (let i = 0; i < entries.length; i++) {
    const entry = entries[i];
    if (
      !Array.isArray(entry) ||
      (entry.length !== 2 && entry.length !== 3)
    ) {
      invalid(name, `text run ${i}`);
    }
    validateSlot(name, entry[0], `text run ${i} slot`);
    validateParts(name, entry[1], `text run ${i} parts`);
    if (entry[2] !== undefined && entry[2] !== 1) {
      invalid(name, `text run ${i} raw flag`);
    }
  }
}

function validateAttributes(
  name: string,
  value: unknown,
  options: ConditionOptions,
): void {
  const entries = optionalArray(name, value, 'attributes');
  for (let i = 0; i < entries.length; i++) {
    const entry = entries[i];
    if (!Array.isArray(entry) || entry.length !== 3 || typeof entry[0] !== 'string') {
      invalid(name, `attribute ${i}`);
    }
    switch (entry[1]) {
      case 0:
      case 1:
        if (typeof entry[2] !== 'string') invalid(name, `attribute ${i} value`);
        break;
      case 2:
        normalizeCondition(name, entry[2], options);
        break;
      case 3:
        validateParts(name, entry[2], `attribute ${i} parts`);
        break;
      default:
        invalid(name, `attribute ${i} kind`);
    }
  }
}

function validateAttributeGroups(name: string, value: unknown): void {
  const entries = optionalArray(name, value, 'attribute groups');
  for (let i = 0; i < entries.length; i++) {
    const entry = entries[i];
    if (!Array.isArray(entry) || entry.length !== 3) {
      invalid(name, `attribute group ${i}`);
    }
    validateNodePath(name, entry[0], `attribute group ${i} target`);
    if (!isIndex(entry[1]) || !isIndex(entry[2])) {
      invalid(name, `attribute group ${i} range`);
    }
  }
}

function validateConditionals(
  name: string,
  value: unknown,
  blockCount: number,
  options: ConditionOptions,
): void {
  const entries = optionalArray(name, value, 'conditions');
  for (let i = 0; i < entries.length; i++) {
    const entry = entries[i];
    if (!Array.isArray(entry) || entry.length !== 3) {
      throw new Error(`[WebUI] Invalid compiled condition tuple for <${name}>.`);
    }
    normalizeCondition(name, entry[0], options);
    validateBlockIndex(name, entry[1], blockCount, 'condition');
    validateSlot(name, entry[2], 'condition slot');
  }
}

function validateRepeats(name: string, value: unknown, blockCount: number): void {
  const entries = optionalArray(name, value, 'repeats');
  for (let i = 0; i < entries.length; i++) {
    const entry = entries[i];
    if (
      !Array.isArray(entry) ||
      (entry.length !== 4 && entry.length !== 5) ||
      typeof entry[0] !== 'string' ||
      typeof entry[1] !== 'string'
    ) {
      invalid(name, `repeat ${i}`);
    }
    validateBlockIndex(name, entry[2], blockCount, 'repeat');
    validateSlot(name, entry[3], `repeat ${i} slot`);
    if (entry[4] !== undefined && typeof entry[4] !== 'string') {
      invalid(name, `repeat ${i} key path`);
    }
  }
}

function validateEventGroups(name: string, value: unknown): void {
  const groups = optionalArray(name, value, 'event groups');
  for (let i = 0; i < groups.length; i++) {
    const group = groups[i];
    if (!Array.isArray(group) || group.length !== 2 || typeof group[0] !== 'string') {
      invalid(name, `event group ${i}`);
    }
    const bindings = requiredArray(name, group[1], `event group ${i} bindings`);
    for (let j = 0; j < bindings.length; j++) {
      const binding = bindings[j];
      if (
        !Array.isArray(binding) ||
        (binding.length !== 3 && binding.length !== 4) ||
        typeof binding[0] !== 'string'
      ) {
        invalid(name, `event group ${i} binding ${j}`);
      }
      validateEventArgs(name, binding[1], `event group ${i} binding ${j}`);
      validateNodePath(name, binding[2], `event group ${i} binding ${j} target`);
      if (binding[3] !== undefined && binding[3] !== 1) {
        invalid(name, `event group ${i} binding ${j} event flag`);
      }
    }
  }
}

function validateRootFields(name: string, root: Record<string, unknown>): void {
  if (root.sa !== undefined && typeof root.sa !== 'string') {
    invalid(name, 'adopted stylesheet');
  }
  validateStringArray(name, root.tr, 'template roots');
  validateStringArray(name, root.ta, 'observed attributes');
  if (root.sd !== undefined && root.sd !== 1) {
    invalid(name, 'shadow DOM flag');
  }
  if (
    root.th !== undefined &&
    root.th !== true &&
    root.th !== false &&
    root.th !== 1
  ) {
    invalid(name, 'template host flag');
  }

  const rootEvents = optionalArray(name, root.re, 'root events');
  for (let i = 0; i < rootEvents.length; i++) {
    const event = rootEvents[i];
    if (
      !Array.isArray(event) ||
      event.length !== 3 ||
      typeof event[0] !== 'string' ||
      typeof event[1] !== 'string'
    ) {
      invalid(name, `root event ${i}`);
    }
    validateEventArgs(name, event[2], `root event ${i}`);
  }
}

function normalizeCondition(
  name: string,
  value: unknown,
  options: ConditionOptions,
): void {
  if (
    !Array.isArray(value) ||
    value.length !== 2 ||
    !Array.isArray(value[1]) ||
    value[1].some(path => typeof path !== 'string')
  ) {
    throw new Error(`[WebUI] Invalid compiled condition metadata for <${name}>.`);
  }
  const first = value[0];
  if (typeof first === 'function') {
    if (!options.allowInlineFunctions) {
      throw new Error(
        `[WebUI] Component asset condition for <${name}> must reference an asset-local closure index.`,
      );
    }
    return;
  }
  if (typeof first !== 'number' || !isIndex(first)) {
    throw new Error(`[WebUI] Invalid condition closure index ${String(first)} for <${name}>.`);
  }
  const fn = options.functions[first];
  if (typeof fn !== 'function') {
    throw new Error(`[WebUI] Missing condition closure ${first} for <${name}>.`);
  }
  options.pending.push({ condition: value, fn });
}

function validateBlockIndex(
  name: string,
  value: unknown,
  blockCount: number,
  kind: 'condition' | 'repeat',
): void {
  if (!isIndex(value) || value >= blockCount) {
    throw new Error(
      `[WebUI] Template ${kind} block index ${String(value)} for <${name}> does not reference one of ${blockCount} blocks.`,
    );
  }
}

function validateSlot(name: string, value: unknown, field: string): void {
  if (
    !Array.isArray(value) ||
    (value.length !== 2 && value.length !== 3)
  ) {
    invalid(name, field);
  }
  validateNodePath(name, value[0], `${field} parent path`);
  if (!isIndex(value[1]) || (value[2] !== undefined && !isIndex(value[2]))) {
    invalid(name, field);
  }
}

function validateNodePath(name: string, value: unknown, field: string): void {
  if (!Array.isArray(value) || value.some(index => !isIndex(index))) {
    invalid(name, field);
  }
}

function validateParts(name: string, value: unknown, field: string): void {
  const parts = requiredArray(name, value, field);
  for (let i = 0; i < parts.length; i++) {
    const part = parts[i];
    if (
      typeof part !== 'string' &&
      (!Array.isArray(part) || part.length !== 1 || typeof part[0] !== 'string')
    ) {
      invalid(name, `${field} entry ${i}`);
    }
  }
}

function validateEventArgs(name: string, value: unknown, field: string): void {
  const args = requiredArray(name, value, `${field} arguments`);
  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (!Array.isArray(arg)) invalid(name, `${field} argument ${i}`);
    const tag = arg[0];
    const valid = (tag === 'e' || tag === 'z')
      ? arg.length === 1
      : (tag === 'p' || tag === 's')
        ? arg.length === 2 && typeof arg[1] === 'string'
        : (tag === 'n' || tag === 'b')
          ? arg.length === 2 && typeof arg[1] === 'number' && Number.isFinite(arg[1])
          : false;
    if (!valid) invalid(name, `${field} argument ${i}`);
  }
}

function validateStringArray(name: string, value: unknown, field: string): void {
  if (value === undefined) return;
  const entries = requiredArray(name, value, field);
  if (entries.some(entry => typeof entry !== 'string')) invalid(name, field);
}

function templateBlock(
  name: string,
  value: unknown,
  field: string,
): Record<string, unknown> {
  if (
    typeof value !== 'object' ||
    value === null ||
    Array.isArray(value) ||
    typeof (value as Record<string, unknown>).h !== 'string'
  ) {
    throw new Error(
      `[WebUI] Template ${field} for <${name}> must contain an HTML string in "h".`,
    );
  }
  return value as Record<string, unknown>;
}

function optionalArray(name: string, value: unknown, field: string): unknown[] {
  if (value === undefined) return [];
  return requiredArray(name, value, field);
}

function requiredArray(name: string, value: unknown, field: string): unknown[] {
  if (!Array.isArray(value)) invalid(name, field);
  return value;
}

function isIndex(value: unknown): value is number {
  return typeof value === 'number'
    && Number.isSafeInteger(value)
    && value >= 0;
}

function invalid(name: string, field: string): never {
  throw new Error(`[WebUI] Invalid ${field} for <${name}>.`);
}
