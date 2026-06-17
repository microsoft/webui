// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { buildFixtureEntries } from '@microsoft/webui-test-support/fixture-build';
import { renderFixtures } from '@microsoft/webui-test-support/fixture-render';
import { startFixtureServer } from '@microsoft/webui-test-support/fixture-server';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const fixturesRoot = resolve(here, 'fixtures');
const outDir = resolve(fixturesRoot, 'dist');
const tsconfig = resolve(here, '..', 'tsconfig.test.json');
const port = Number(process.env.PORT ?? 39101);

// Build and render fixtures that have real WebUI templates (src/index.html).
// The rendered HTML includes template metadata, condition closures, hydration markers, and inventory.
const rendered = renderFixtures({ fixturesRoot });

// Bundle element.ts entrypoints (component class definitions) for client-side.
await buildFixtureEntries({
  fixturesRoot,
  entryFileName: 'element.ts',
  outDir,
  tsconfig,
  emptyMessage: `No fixture entry points found in ${fixturesRoot}`,
});

startFixtureServer({
  name: 'webui-framework',
  fixturesRoot,
  port,
  handleRequest({ url, send, serveStatic }) {
    if (url.pathname === '/') {
      send(200, 'webui-framework fixture server');
      return true;
    }

    // Serve pipeline-rendered fixture HTML.
    // URL pattern: /<fixture-name>/fixture.html
    const match = url.pathname.match(/^\/([^/]+)\/fixture\.html$/);
    if (match) {
      const fixture = rendered.get(match[1]);
      if (fixture) {
        send(200, fixture.html, 'text/html; charset=utf-8');
        return true;
      }
    }

    // Fall back to static files (bundled JS, legacy fixtures, etc.)
    return serveStatic(url.pathname);
  },
});
