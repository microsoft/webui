// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect } from '@playwright/test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { binary, fixture, removeFixtureRoot, unusedPort, write } from './native-fixture.js';

test('serve watches generated JSON includes, replacements and newly referenced missing files', async t => {
  const f = fixture();
  const port = await unusedPort();
  const first = path.join(f.site, 'target/webui-press-state/first.json');
  const second = path.join(f.site, 'target/webui-press-state/new/nested/second.json');
  const markdown = path.join(f.site, 'content/doc.md');
  const source = (name: string) => `---
stateFile: ../target/webui-press-state/${name}.json
state:
  catalogMessage: Inline example
---

<test-catalog-button label="Preview" :count="{{count}}"></test-catalog-button>
<test-catalog-text></test-catalog-text>

<script type="module" bundle>
import '@fixture/catalog/button.js';
</script>
`;
  write(first, '{"count":4}');
  write(markdown, source('first'));
  write(f.configFile, JSON.stringify({ ...f.config, regions: {} }));
  const child = spawn(binary, ['serve', '--show=content', '--port', String(port)], {
    cwd: f.site, stdio: ['ignore', 'pipe', 'pipe'],
  });
  let output = '';
  child.stdout.on('data', data => { output += data; });
  child.stderr.on('data', data => { output += data; });
  const exited = once(child, 'exit');
  t.after(async () => {
    child.kill('SIGINT');
    await exited;
    removeFixtureRoot(f.root);
  });
  const url = `http://127.0.0.1:${port}/fixture/doc/`;
  const rendered = async () => {
    assert.equal(child.exitCode, null, output);
    try {
      return await (await fetch(url, { headers: { Connection: 'close' } })).text();
    } catch (error) {
      if (error instanceof TypeError && error.cause instanceof Error &&
          'code' in error.cause && error.cause.code === 'ECONNREFUSED') return '';
      throw error;
    }
  };
  await expect.poll(rendered, { timeout: 60_000 }).toContain('Preview 4');
  assert.ok((await rendered()).includes('Inline example'));
  write(first, '{"count":5}');
  await expect.poll(rendered, { timeout: 30_000 }).toContain('Preview 5');
  write(`${first}.tmp`, '{"count":6}');
  fs.renameSync(`${first}.tmp`, first);
  await expect.poll(rendered, { timeout: 30_000 }).toContain('Preview 6');
  write(markdown, source('new/nested/second'));
  await expect.poll(() => output, { timeout: 30_000 }).toContain('cannot resolve stateFile');
  assert.ok((await rendered()).includes('Preview 6'), 'failed includes preserve the previous output');
  const errors = () => output.match(/cannot resolve stateFile/g)?.length ?? 0;
  let previousErrors = errors();
  fs.mkdirSync(path.dirname(path.dirname(second)));
  await expect.poll(errors, { timeout: 30_000 }).toBeGreaterThan(previousErrors);
  previousErrors = errors();
  fs.mkdirSync(path.dirname(second));
  await expect.poll(errors, { timeout: 30_000 }).toBeGreaterThan(previousErrors);
  write(second, '{"count":7}');
  await expect.poll(rendered, { timeout: 30_000 }).toContain('Preview 7');
  write(second, '{"count":8}');
  await expect.poll(rendered, { timeout: 30_000 }).toContain('Preview 8');
  previousErrors = errors();
  fs.unlinkSync(second);
  await expect.poll(errors, { timeout: 30_000 }).toBeGreaterThan(previousErrors);
  write(second, '{"count":9}');
  await expect.poll(rendered, { timeout: 30_000 }).toContain('Preview 9');
  previousErrors = errors();
  fs.unlinkSync(second);
  fs.rmdirSync(path.dirname(second));
  await expect.poll(errors, { timeout: 30_000 }).toBeGreaterThan(previousErrors);
  write(second, '{"count":10}');
  await expect.poll(rendered, { timeout: 30_000 }).toContain('Preview 10');
  previousErrors = errors();
  write(markdown, source('rapid/one/two/third'));
  await expect.poll(errors, { timeout: 30_000 }).toBeGreaterThan(previousErrors);
  write(path.join(f.site, 'target/webui-press-state/rapid/one/two/third.json'), '{"count":11}');
  await expect.poll(rendered, { timeout: 30_000 }).toContain('Preview 11');
});
