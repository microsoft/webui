// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { readdirSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

test('shipped router modules have no WebUI Framework dependency', () => {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', 'src');
  const pending = [root];
  const violations: string[] = [];
  while (pending.length > 0) {
    const directory = pending.pop()!;
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) {
        pending.push(path);
      } else if (
        entry.name.endsWith('.ts')
        && !entry.name.endsWith('.test.ts')
        && readFileSync(path, 'utf8').includes('@microsoft/webui-framework')
      ) {
        violations.push(path);
      }
    }
  }
  assert.deepEqual(violations, []);
});
