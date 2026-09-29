// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const fixture = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(fixture, '../../../../..');
const require = createRequire(path.join(root, 'packages/webui-desktop/package.json'));
const { build } = require('esbuild');
const output = process.argv[2];
if (!output) throw new Error('Expected an output path for local native fixture JS');
await build({
  entryPoints: [path.join(fixture, 'local-renderer.ts')],
  outfile: output,
  bundle: true,
  format: 'esm',
  platform: 'browser',
  target: 'es2022',
  nodePaths: [path.join(root, 'packages/webui-desktop/node_modules')],
  plugins: [{
    name: 'embedded-local-sdk-runtime',
    setup(builder) {
      builder.onResolve({ filter: /^@microsoft\/webui-desktop(?:\/native)?$/ }, () => ({
        path: '/_webui/ipc/local-runtime.js', external: true,
      }));
    },
  }],
});
