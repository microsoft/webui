// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, mkdir, writeFile, realpath, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';

const runner = fileURLToPath(new URL('./native-build.mjs', import.meta.url));

async function fixture(t) {
  const root = await realpath(await mkdtemp(path.join(tmpdir(), 'webui-native-build-')));
  t.after(() => rm(root, { recursive: true, force: true }));
  const tools = path.join(root, 'prepared tools');
  await mkdir(tools);
  const runtime = path.join(tools, 'runtime.node');
  const builder = path.join(tools, 'builder.node');
  await writeFile(runtime, '');
  await writeFile(builder, '');
  const env = {
    ...process.env,
    PATH: '',
    WEBUI_ADDON_PATH: runtime,
    WEBUI_BUILD_ADDON_PATH: builder,
    WEBUI_BINARY_PATH: process.execPath,
  };
  const run = (args, overrides = {}) => spawnSync(process.execPath, [runner, ...args], {
    cwd: root,
    env: { ...env, ...overrides },
    encoding: 'utf8',
  });
  return { root, runtime, builder, run };
}

test('prepared clients and CLI execute without Cargo and preserve paths, cwd, and arguments', async t => {
  const { root, runtime, builder, run } = await fixture(t);
  const script = path.join(root, 'client with spaces.mjs');
  await writeFile(script, `
    console.log(JSON.stringify({
      cwd: process.cwd(), args: process.argv.slice(2),
      runtime: process.env.WEBUI_ADDON_PATH,
      builder: process.env.WEBUI_BUILD_ADDON_PATH
    }));
  `);
  for (const command of ['client', 'cli']) {
    const result = run([command, script, 'argument with spaces']);
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(JSON.parse(result.stdout), {
      cwd: root, args: ['argument with spaces'], runtime, builder,
    });
  }
  const prepared = run(['addons']);
  assert.equal(prepared.status, 0, prepared.stderr);
});

test('prepared commands propagate failure and reject missing or incomplete artifacts without rebuilding', async t => {
  const { root, run } = await fixture(t);
  const script = path.join(root, 'failure.mjs');
  await writeFile(script, 'process.exitCode = 23;');
  for (const command of ['client', 'cli']) {
    assert.equal(run([command, script]).status, 23);
  }
  for (const overrides of [
    { WEBUI_BUILD_ADDON_PATH: '' },
    { WEBUI_ADDON_PATH: path.join(root, 'missing.node') },
    { WEBUI_BUILD_ADDON_PATH: root },
    { WEBUI_BUILD_ADDON_PATH: 'relative.node' },
  ]) {
    const result = run(['client', script], overrides);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /WEBUI_(BUILD_)?ADDON_PATH/);
    assert.doesNotMatch(result.stderr, /spawnSync cargo/);
  }
  const missingCli = run(['cli', script], { WEBUI_BINARY_PATH: path.join(root, 'missing-cli') });
  assert.equal(missingCli.status, 1);
  assert.match(missingCli.stderr, /WEBUI_BINARY_PATH/);
});

test('standalone commands require Cargo instead of silently using an unprepared tool', async t => {
  const { run } = await fixture(t);
  for (const args of [['addons'], ['client', 'client.mjs'], ['cli', '--version']]) {
    const result = run(args, {
      WEBUI_ADDON_PATH: '',
      WEBUI_BUILD_ADDON_PATH: '',
      WEBUI_BINARY_PATH: '',
    });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /cargo.*ENOENT/);
  }
});
