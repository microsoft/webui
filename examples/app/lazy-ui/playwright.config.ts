// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests',
  timeout: 30_000,
  use: {
    baseURL: 'http://127.0.0.1:3011',
    screenshot: 'only-on-failure',
    trace: 'retain-on-failure',
  },
  webServer: [
    {
      command: 'pnpm run start:api',
      url: 'http://127.0.0.1:3021/health',
      reuseExistingServer: true,
      timeout: 30_000,
    },
    {
      command: 'pnpm run build && pnpm run start:server',
      url: 'http://127.0.0.1:3011/index.js',
      reuseExistingServer: true,
      timeout: 180_000,
      env: { WEBUI_NO_WATCH: '1' },
    },
  ],
  projects: [{ name: 'chromium', use: { browserName: 'chromium' } }],
});
