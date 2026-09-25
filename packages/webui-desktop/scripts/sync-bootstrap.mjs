// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const [mode, target] = process.argv.slice(2);
if ((mode !== '--check' && mode !== '--write') || !target) {
  throw new Error('Usage: node scripts/sync-bootstrap.mjs --check|--write SDK_GENERATED_DIRECTORY');
}
const dist = fileURLToPath(new URL('../dist/', import.meta.url));
const destination = resolve(target);
if (mode === '--write') await mkdir(destination, { recursive: true });
for (const name of ['native-bootstrap.js', 'desktop-runtime.js']) {
  const source = await readFile(resolve(dist, name));
  if (mode === '--write') {
    await writeFile(resolve(destination, name), source);
  } else {
    const installed = await readFile(resolve(destination, name));
    if (!source.equals(installed)) throw new Error(`Generated IPC asset differs: ${name}. Run the explicit --write sync command.`);
  }
}
