// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { defineConfig } from '@playwright/test';

const port = 39101;

export default defineConfig({
  testDir: './tests/fixtures',
  testMatch: '**/*.spec.ts',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  timeout: 30_000,
  outputDir: 'test-results',
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    screenshot: 'only-on-failure',
    launchOptions: {
      args: ['--enable-blink-features=DeclarativeCSSModules'],
    },
  },
  webServer: {
    command: 'node --experimental-strip-types ./tests/server.ts',
    port,
    reuseExistingServer: !process.env.CI,
  },
});
