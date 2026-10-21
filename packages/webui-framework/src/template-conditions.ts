// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/** Coerce a resolved JSON condition operand using the server's truthiness rules. */
export function isConditionTruthy(value: unknown): boolean {
  if (!value || typeof value !== 'object') return !!value;
  return (Array.isArray(value) ? value.length : Object.keys(value).length) > 0;
}
