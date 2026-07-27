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

// Bundle element.ts entrypoints (component class definitions) for client-side.
const projectionManifest = await buildFixtureEntries({
  fixturesRoot,
  entryFileName: 'element.ts',
  outDir,
  tsconfig,
  emptyMessage: `No fixture entry points found in ${fixturesRoot}`,
  extraBuilds: [{
    entryPoints: [resolve(here, 'static-host.ts')],
    bundle: true,
    format: 'iife',
    outfile: resolve(outDir, 'static-host.js'),
    platform: 'browser',
    target: 'es2022',
    supported: { 'import-attributes': true },
    tsconfig,
  }],
});

// Render only after the client bundle has produced exact projection metadata.
const rendered = renderFixtures({
  fixturesRoot,
  projectionManifest,
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

    // Serve static files for bundled JS, authored fixture HTML, and assets.
    return serveStatic(url.pathname);
  },
});
