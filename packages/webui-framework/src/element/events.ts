// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Parse per-element SSR hydration event markers.
 *
 * The current WebUI SSR contract emits one `data-ev` marker per element. Its
 * value is the number of consecutive entries in the template metadata `e[]`
 * array that belong to that element.
 *
 * Returns `null` when the markers do not match the count-based contract.
 */
export function readHydrationEventCounts(
  markerValues: readonly (string | null)[],
  eventCount: number,
): number[] | null {
  const counts: number[] = [];
  let total = 0;

  for (const value of markerValues) {
    if (!value) {
      return null;
    }

    const count = Number.parseInt(value, 10);
    if (!Number.isSafeInteger(count) || count <= 0) {
      return null;
    }

    total += count;
    if (total > eventCount) {
      return null;
    }
    counts.push(count);
  }

  return total === eventCount ? counts : null;
}
