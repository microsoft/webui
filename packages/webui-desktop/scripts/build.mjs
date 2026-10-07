// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { build } from 'esbuild';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const localCarrier = process.argv.slice(2).includes('--local-carrier');
if (process.argv.slice(2).some(argument => argument !== '--local-carrier')) {
  throw new Error('Usage: node scripts/build.mjs [--local-carrier]');
}
const options = {
  bundle: true,
  platform: 'browser',
  target: 'es2022',
  minify: true,
  legalComments: 'none',
  charset: 'utf8',
  banner: { js: '// Copyright (c) Microsoft Corporation.\n// Licensed under the MIT license.\n' },
};
await Promise.all((localCarrier ? [true] : [false, true]).flatMap(native => {
  const prefix = native ? 'local-' : '';
  return [
    build({ ...options, define: { __WEBUI_NATIVE_CARRIER__: String(native) },
      entryPoints: [`${root}src/native-entry.ts`], format: 'iife',
      outfile: `${root}dist/${prefix}native-bootstrap.js` }),
    build({ ...options, ...(native ? { define: { __WEBUI_NATIVE_CARRIER__: 'true' } } : {}),
      entryPoints: [`${root}src/${native ? 'local-index.ts' : 'index.ts'}`], format: 'esm',
      outfile: `${root}dist/${prefix}desktop-runtime.js` }),
  ];
}));
