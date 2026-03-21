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
    baseURL: `http://${host}:3004`,
    screenshot: 'only-on-failure',
  },
  projects: [
    {
      name: 'chromium',
      use: { browserName: 'chromium' },
      grepInvert: /mobile layout/,
    },
    {
      name: 'mobile',
      use: {
        browserName: 'chromium',
        viewport: { width: 390, height: 844 },
        isMobile: true,
        hasTouch: true,
      },
      grep: /mobile layout/,
    },
  ],
  // Server must be started separately before running tests:
  //   pnpm start:server  (cargo run on port 3004)
});
