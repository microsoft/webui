// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Explicit Linux-only generated bootstrap drift gate. Never bundle this
// optional transport into the default desktop entry points.
import { build } from 'esbuild';
import { readFile, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';

const mode = process.argv[2];
if (mode !== '--write' && mode !== '--check') {
  throw new Error('Usage: node scripts/linux-assets.mjs --write|--check');
}
const root = fileURLToPath(new URL('../', import.meta.url));
const generated = resolve(root, '../../crates/webui-desktop/src/generated/ipc');
for (const [entry, output] of [
  ['linux-local-entry.ts', 'linux-local-entry.js'],
  ['linux-local-mediator.ts', 'linux-local-mediator.js'],
]) {
  const result = await build({
    entryPoints: [resolve(root, 'src', entry)],
    bundle: true, platform: 'browser', format: 'iife', target: 'es2022',
    minify: true, legalComments: 'none', charset: 'utf8', write: false,
    banner: { js: '// Copyright (c) Microsoft Corporation.\n// Licensed under the MIT license.\n' },
  });
  const bytes = result.outputFiles[0]?.contents;
  if (!bytes) throw new Error(`Missing generated Linux asset: ${entry}`);
  const destination = resolve(generated, output);
  if (mode === '--write') await writeFile(destination, bytes);
  else if (!Buffer.from(bytes).equals(await readFile(destination))) {
    throw new Error(`Linux IPC asset differs: ${output}. Run linux-assets.mjs --write.`);
  }
}
