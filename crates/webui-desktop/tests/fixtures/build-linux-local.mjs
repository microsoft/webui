// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { createRequire } from 'node:module';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const fixture = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(fixture, '../../../..');
const require = createRequire(path.join(root, 'packages/webui-desktop/package.json'));
const { build } = require('esbuild');
const check = process.argv[2] === '--check';
const output = check
  ? (process.argv[3] ?? path.join(fixture, 'linux-local-renderer.js'))
  : process.argv[2];
if (!output) throw new Error('Expected Linux native IPC fixture output path or --check');
const result = await build({
  entryPoints: [path.join(fixture, 'linux-local-renderer.ts')],
  outfile: output,
  write: !check,
  bundle: true,
  format: 'esm',
  platform: 'browser',
  target: 'es2022',
  banner: { js: '// Copyright (c) Microsoft Corporation.\n// Licensed under the MIT license.\n' },
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
if (check) {
  const rendered = result.outputFiles[0]?.contents;
  if (!rendered || !Buffer.from(rendered).equals(await readFile(output))) {
    throw new Error('Linux native IPC fixture asset drifted; rebuild with build-linux-local.mjs OUTPUT');
  }
}
