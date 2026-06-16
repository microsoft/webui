// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests',
  timeout: 30_000,
  use: {
    baseURL: 'http://127.0.0.1:4175',
  },
  webServer: {
    command: 'node dist-scripts/serve.js --port 4175',
    url: 'http://127.0.0.1:4175',
    reuseExistingServer: !process.env['CI'],
  },
  projects: [
    { name: 'chromium', use: { browserName: 'chromium' } },
  ],
});
