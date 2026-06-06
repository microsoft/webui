// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests',
  snapshotPathTemplate:
    '{snapshotDir}/{testFileDir}/{testFileName}-snapshots/{arg}-{projectName}{ext}',
  timeout: 30_000,
  use: {
    baseURL: 'http://127.0.0.1:3004',
    screenshot: 'only-on-failure',
    launchOptions: {
      // The Module CSS strategy emits `<style type="module" specifier="…">`
      // pairs with `shadowrootadoptedstylesheets` on declarative shadow roots.
      // This relies on the Declarative CSS Modules proposal, which is currently
      // behind a Blink feature flag.
      args: ['--enable-blink-features=DeclarativeCSSModules'],
    },
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
