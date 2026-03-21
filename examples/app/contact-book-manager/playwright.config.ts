// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { defineConfig } from '@playwright/test';

const host = process.env.WEBUI_TEST_HOST || '127.0.0.1';

export default defineConfig({
  testDir: './tests',
  snapshotPathTemplate:
    '{snapshotDir}/{testFileDir}/{testFileName}-snapshots/{arg}-{projectName}{ext}',
  timeout: 30_000,
  use: {
    baseURL: `http://${host}:3003`,
    screenshot: 'only-on-failure',
  },
  projects: [
    { name: 'chromium', use: { browserName: 'chromium' } },
  ],
  // Servers must be started separately before running tests:
  //   pnpm start:api    (Express on 3013)
  //   pnpm start:server (WebUI on 3003)
});
