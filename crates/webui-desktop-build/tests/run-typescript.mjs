// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { build } from '../../../packages/webui-desktop/node_modules/esbuild/lib/main.js';

const testRoot = new URL('./', import.meta.url);
execFileSync(process.execPath, [
  fileURLToPath(new URL('../../../packages/webui-desktop/node_modules/typescript/bin/tsc', import.meta.url)),
  '-p', fileURLToPath(new URL('tsconfig.json', testRoot)),
], { stdio: 'inherit' });
const result = await build({
  entryPoints: [fileURLToPath(new URL('typescript.ts', testRoot))],
  bundle: true, write: false, platform: 'node', format: 'esm', target: 'es2022',
  tsconfig: fileURLToPath(new URL('tsconfig.json', testRoot)),
  alias: {
    '@microsoft/webui-desktop': fileURLToPath(new URL('../../../packages/webui-desktop/src/index.ts', import.meta.url)),
    '@bufbuild/protobuf/wire': fileURLToPath(new URL('../../../packages/webui-desktop/node_modules/@bufbuild/protobuf/dist/esm/wire/index.js', import.meta.url)),
  },
  define: { 'import.meta.url': JSON.stringify(new URL('typescript.ts', testRoot).href) },
});
try {
  await import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].contents).toString('base64')}`);
} catch (error) {
  // A data-URL stack repeats the entire bundle. Keep failed checks actionable.
  console.error(error instanceof Error ? `${error.name}: ${error.message}` : String(error));
  process.exitCode = 1;
}
