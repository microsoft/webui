// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const fixture = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(fixture, '../../../../..');
const require = createRequire(path.join(root, 'packages/webui-desktop/package.json'));
const { build } = require('esbuild');
if (!process.argv[2]) throw new Error('Expected an immutable run output directory');
await build({
  entryPoints: [path.join(fixture, 'renderer.ts')],
  outfile: path.join(process.argv[2], 'assets/fixture.js'),
  bundle: true,
  format: 'esm',
  platform: 'browser',
  target: 'es2022',
  nodePaths: [path.join(root, 'packages/webui-desktop/node_modules')],
  plugins: [{
    name: 'sdk-reserved-runtime',
    setup(builder) {
      builder.onResolve({ filter: /^@microsoft\/webui-desktop$/ }, () => ({
        path: '/_webui/ipc/runtime.js', external: true,
      }));
    },
  }],
});
