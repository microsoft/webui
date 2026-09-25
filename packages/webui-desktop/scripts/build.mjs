// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { build } from 'esbuild';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const options = {
  bundle: true,
  platform: 'browser',
  target: 'es2022',
  minify: true,
  legalComments: 'none',
  charset: 'utf8',
  banner: { js: '// Copyright (c) Microsoft Corporation.\n// Licensed under the MIT license.\n' },
};
await Promise.all([
  build({ ...options, entryPoints: [`${root}src/native-entry.ts`], format: 'iife', outfile: `${root}dist/native-bootstrap.js` }),
  build({ ...options, entryPoints: [`${root}src/index.ts`], format: 'esm', outfile: `${root}dist/desktop-runtime.js` }),
]);
