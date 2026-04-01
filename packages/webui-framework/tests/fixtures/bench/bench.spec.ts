// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

interface BenchResult {
  iterations: number;
  bindings: { text: number; attr: number; boolAttr: number; total: number };
  singlePropMs: number;
  singlePropPerUpdate: number;
  allPropsMs: number;
  allPropsPerUpdate: number;
}

for (const mode of ['light', 'shadow'] as const) {
test.describe(`bench fixture [${mode} DOM]`, () => {
  test.beforeEach(async ({ page }) => {
    const file = mode === 'light' ? 'fixture.html' : 'fixture-shadow.html';
    await page.goto(`/bench/${file}`);
    await page.waitForSelector('test-bench');
  });

  test('measures update performance baseline', async ({ page }) => {
    await page.locator('test-bench .run').click();

    await page.waitForFunction(
      () => (window as unknown as Record<string, unknown>).__benchResult != null,
    );

    const result = await page.evaluate(
      () => (window as unknown as Record<string, unknown>).__benchResult as BenchResult,
    );

    // Log results for comparison
    console.log('\n=== WebUI Framework Update Benchmark ===');
    console.log(`Bindings: ${result.bindings.total} (${result.bindings.text} text, ${result.bindings.attr} attr, ${result.bindings.boolAttr} bool)`);
    console.log(`Iterations: ${result.iterations}`);
    console.log(`Single prop mutation: ${result.singlePropMs}ms total, ${result.singlePropPerUpdate}ms/update`);
    console.log(`All props mutation:   ${result.allPropsMs}ms total, ${result.allPropsPerUpdate}ms/update`);
    console.log(`Ratio (single/all):   ${Math.round((result.singlePropPerUpdate / result.allPropsPerUpdate) * 100)}%`);
    console.log('=========================================\n');

    // Sanity checks — don't assert specific times (hardware-dependent),
    // just verify the benchmark ran and produced reasonable data.
    expect(result.iterations).toBe(10_000);
    expect(result.bindings.total).toBe(65);
    expect(result.singlePropMs).toBeGreaterThan(0);
    expect(result.allPropsMs).toBeGreaterThan(0);

    // The key metric for Phase 5: if per-path tracking works,
    // single-prop should be significantly faster than all-props.
    // Before Phase 5, they should be roughly similar (both walk all bindings).
    // After Phase 5, single-prop should be ~10x faster.
    //
    // We don't assert this ratio yet — this test captures the baseline.
  });
});
}
