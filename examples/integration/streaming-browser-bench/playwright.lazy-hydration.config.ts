// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests',
  testMatch: '**/lazy_hydration_matrix.spec.ts',
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: 0,
  workers: 1,
  timeout: 600_000,
  reporter: 'list',
  use: {
    headless: true,
    viewport: { width: 1280, height: 720 },
    launchOptions: {
      args: ['--enable-precise-memory-info'],
    },
  },
});
