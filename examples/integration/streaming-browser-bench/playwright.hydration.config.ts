// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { defineConfig } from '@playwright/test';

/**
 * Hydration-matrix config.
 *
 * The hydration bench drives the DOM directly via `page.setContent` +
 * `page.addScriptTag`, so it needs no HTTP server (unlike the transport bench in
 * `playwright.config.ts`). It launches Chromium with `--enable-precise-memory-info`
 * so `performance.memory.usedJSHeapSize` reports real peak-heap deltas.
 */
export default defineConfig({
  testDir: './tests',
  testMatch: '**/hydration_matrix.spec.ts',
  fullyParallel: false, // measurements must not contend
  forbidOnly: !!process.env.CI,
  retries: 0,
  workers: 1, // serial execution -> clean per-run measurements
  timeout: 600_000,
  reporter: 'list',
  use: {
    headless: true,
    launchOptions: {
      args: ['--enable-precise-memory-info'],
    },
  },
});
