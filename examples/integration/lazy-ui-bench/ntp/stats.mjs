// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Summary statistics and a seeded bootstrap for comparing medians.

/** Median (upper of the two middle values for even counts, as in triggers.mjs), p90, and range. */
export function stats(values) {
  const sorted = values.filter(Number.isFinite).sort((a, b) => a - b);
  if (sorted.length === 0) return undefined;
  const at = p => sorted[Math.min(sorted.length - 1, Math.max(0, Math.ceil(p * sorted.length) - 1))];
  return { n: sorted.length, median: at(0.5), p90: at(0.9), min: sorted[0], max: sorted.at(-1) };
}

function median(sorted) {
  const n = sorted.length;
  return n % 2 ? sorted[(n - 1) / 2] : (sorted[n / 2 - 1] + sorted[n / 2]) / 2;
}

function xorshift(seed) {
  let state = seed >>> 0;
  return () => {
    state ^= state << 13; state >>>= 0;
    state ^= state >>> 17;
    state ^= state << 5; state >>>= 0;
    return state / 4294967296;
  };
}

/**
 * Difference of medians, `a - b`, with a 95% percentile-bootstrap interval.
 * The seed is fixed so a recorded result can be reproduced from its samples.
 */
export function compare(a, b, iterations = 4000) {
  const left = a.filter(Number.isFinite);
  const right = b.filter(Number.isFinite);
  if (left.length === 0 || right.length === 0) return undefined;
  const random = xorshift(0x9e3779b9);
  const draw = values => {
    const sample = new Array(values.length);
    for (let i = 0; i < values.length; i++) sample[i] = values[Math.floor(random() * values.length)];
    return median(sample.sort((x, y) => x - y));
  };
  const differences = new Array(iterations);
  for (let i = 0; i < iterations; i++) differences[i] = draw(left) - draw(right);
  differences.sort((x, y) => x - y);
  return {
    diff: median(left.slice().sort((x, y) => x - y)) - median(right.slice().sort((x, y) => x - y)),
    low: differences[Math.floor(0.025 * iterations)],
    high: differences[Math.ceil(0.975 * iterations) - 1],
  };
}

/** `lower` or `higher` when the interval excludes zero, otherwise `within noise`. */
export function verdict(comparison) {
  if (!comparison) return 'no data';
  return comparison.high < 0 ? 'lower' : comparison.low > 0 ? 'higher' : 'within noise';
}
