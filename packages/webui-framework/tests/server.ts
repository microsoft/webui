// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { buildFixtureEntries } from '@microsoft/webui-test-support/fixture-build';
import { startFixtureServer } from '@microsoft/webui-test-support/fixture-server';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const testsRoot = resolve(here, '..');
const fixturesRoot = resolve(testsRoot, 'fixtures');
const outDir = resolve(fixturesRoot, 'dist');
const tsconfig = resolve(testsRoot, '..', 'tsconfig.test.json');
const port = Number(process.env.PORT ?? 39101);

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

    return serveStatic(url.pathname);
  },
});
