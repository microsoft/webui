// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { defineConfig } from '@playwright/test';

const port = 39102;

export default defineConfig({
  testDir: './tests',
  fullyParallel: true,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  expect: {
    timeout: 10_000,
  },
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    trace: 'on-first-retry',
  },
  webServer: {
    command: `pnpm build:e2e && cargo run -p microsoft-webui-cli -- serve ./tests/fixtures/router-app/src --plugin=webui --servedir ./tests/fixtures/router-app/dist --state ./tests/fixtures/router-app/data/state.json --port ${port}`,
    port,
    reuseExistingServer: true,
  },
});
