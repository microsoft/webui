// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

// This test compiles beside the emitted modules, so it reads what the package
// publishes and what application bundlers consume.
const dist = dirname(fileURLToPath(import.meta.url));
const IGNORE = 'import(/* webpackIgnore: true */ /* @vite-ignore */ ';

test('dynamic imports of runtime URLs tell application bundlers to leave them alone', () => {
  for (const [file, call] of [
    ['element/styles.js', `${IGNORE}specifier, { with: { type: 'css' } })`],
    ['component-asset/loader.js', `${IGNORE}href)`],
  ]) {
    const source = readFileSync(resolve(dist, file), 'utf8');
    assert.ok(source.includes(call), `${file} must emit ${call}`);
  }
});
