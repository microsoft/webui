// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { defineConfig } from '@playwright/test';

const port = 3099;

export default defineConfig({
  testDir: './tests',
  testMatch: '**/*.spec.ts',
  fullyParallel: false, // measurements must not contend
  forbidOnly: !!process.env.CI,
  retries: 0,
  workers: 1, // serial execution → clean per-test measurements
  timeout: 120_000,
  reporter: 'list',
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    headless: true,
    // Disable cache so every navigation is a clean cold load.
    extraHTTPHeaders: {
      'cache-control': 'no-cache',
    },
  },
  webServer: {
    command: `cargo run -p streaming-browser-bench-server --release -- --port ${port}`,
    port,
    timeout: 180_000,
    reuseExistingServer: !process.env.CI,
  },
});
