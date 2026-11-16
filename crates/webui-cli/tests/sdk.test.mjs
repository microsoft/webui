// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { mkdir, mkdtemp, readFile, realpath, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { sdkEnvironment } from '../src/commands/init/templates/common/scripts/sdk.mjs';

const targets = {
  'darwin-arm64': ['aarch64-apple-darwin', 'libwebui_ffi.dylib'],
  'darwin-x64': ['x86_64-apple-darwin', 'libwebui_ffi.dylib'],
  'linux-arm64': ['aarch64-unknown-linux-gnu', 'libwebui_ffi.so'],
  'linux-x64': ['x86_64-unknown-linux-gnu', 'libwebui_ffi.so'],
  'win32-arm64': ['aarch64-pc-windows-msvc', 'webui_ffi.dll'],
  'win32-x64': ['x86_64-pc-windows-msvc', 'webui_ffi.dll'],
};
const platform = `${process.platform}-${process.arch}`;
const [target, library] = targets[platform];
const manifest = await readFile(new URL('../../../Cargo.toml', import.meta.url), 'utf8');
const version = manifest.match(/^version = "([^"]+)"$/m)[1];

async function fixture(t) {
  const root = await realpath(await mkdtemp(path.join(tmpdir(), 'standalone SDK contract ')));
  t.after(() => rm(root, { recursive: true, force: true }));
  const helper = path.join(root, 'sdk.mjs');
  const source = await readFile(process.env.SDK_HELPER_FILE
    ?? fileURLToPath(new URL('../src/commands/init/templates/common/scripts/sdk.mjs', import.meta.url)), 'utf8');
  await writeFile(helper, source.replaceAll('__WEBUI_VERSION__', version));
  const project = path.join(root, 'app');
  await mkdir(project);
  return { root, helper, project };
}

async function sdk(root, additions = {}) {
  const content = {
    'include/webui_ffi.h': 'header',
    [`lib/${library}`]: 'native fixture; never executed',
    'LICENSE': 'MIT',
  };
  if (process.platform === 'win32') content['lib/webui_ffi.dll.lib'] = 'import fixture';
  const files = {};
  for (const [relative, bytes] of Object.entries(content)) {
    const file = path.join(root, relative);
    await mkdir(path.dirname(file), { recursive: true });
    await writeFile(file, bytes);
    files[relative] = createHash('sha256').update(bytes).digest('hex');
  }
  await writeFile(path.join(root, 'sdk.json'), JSON.stringify({
    schemaVersion: 1, version, target, os: process.platform, arch: process.arch,
    libc: process.platform === 'linux' ? 'gnu' : null, files, ...additions,
  }));
  return root;
}

function run(helper, project, explicit) {
  const env = { ...process.env };
  delete env.WEBUI_SDK_DIR;
  if (explicit !== undefined) env.WEBUI_SDK_DIR = explicit;
  return spawnSync(process.execPath, [helper], { cwd: project, env, encoding: 'utf8' });
}

test('standalone SDK metadata needs no npm package identity', async t => {
  const { root, helper, project } = await fixture(t);
  const standalone = await sdk(path.join(root, 'native SDK'));
  const result = run(helper, project, standalone);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout.trim(), await realpath(standalone));
});

test('project-local SDK is separate from node_modules', async t => {
  const { helper, project } = await fixture(t);
  const local = await sdk(path.join(project, '.webui/sdk'));
  const result = run(helper, project);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout.trim(), await realpath(local));
});

test('an npm SDK payload is never selected implicitly', async t => {
  const { helper, project } = await fixture(t);
  const packageName = `@microsoft/webui-${platform}`;
  const packageRoot = path.join(project, 'node_modules', ...packageName.split('/'));
  await sdk(path.join(packageRoot, 'sdk'), { package: packageName });
  await writeFile(path.join(packageRoot, 'package.json'), JSON.stringify({ name: packageName, version }));
  const result = run(helper, project);
  assert.notEqual(result.status, 0, 'SDK unexpectedly resolved from node_modules');
  assert.match(result.stderr, /standalone release archive|Download webui-sdk/);
});

test('invalid or empty explicit SDK never falls back to a valid local SDK', async t => {
  const { root, helper, project } = await fixture(t);
  await sdk(path.join(project, '.webui/sdk'));
  const wrong = await sdk(path.join(root, 'wrong SDK'), { version: '0.0.0' });
  assert.notEqual(run(helper, project, wrong).status, 0);
  assert.notEqual(run(helper, project, '').status, 0);
});

test('SDK validates release, target, OS, architecture, libc and exact inventory', async t => {
  const { root, helper, project } = await fixture(t);
  for (const changes of [
    { schemaVersion: 0 }, { version: '0.0.0' }, { target: 'wrong' },
    { os: 'wrong' }, { arch: 'wrong' }, { files: {} },
    ...(process.platform === 'linux' ? [{ libc: 'musl' }] : []),
  ]) {
    const directory = await sdk(path.join(root, 'invalid'), changes);
    const result = run(helper, project, directory);
    assert.notEqual(result.status, 0, JSON.stringify(changes));
  }
});

test('SDK rejects altered bytes and links escaping its root', async t => {
  const { root, helper, project } = await fixture(t);
  const directory = await sdk(path.join(root, 'SDK'));
  await writeFile(path.join(directory, 'LICENSE'), 'tampered');
  assert.match(run(helper, project, directory).stderr, /integrity check failed/);
  await sdk(directory);
  await rm(path.join(directory, 'LICENSE'));
  await writeFile(path.join(root, 'external'), 'MIT');
  await symlink(path.join(root, 'external'), path.join(directory, 'LICENSE'), 'file');
  assert.match(run(helper, project, directory).stderr, /unsafe file/);
});

test('selected SDK include, link and loader paths precede inherited toolchain paths', () => {
  const root = path.resolve('SDK with spaces');
  const environment = sdkEnvironment(root, {
    CGO_CFLAGS: '-I/stale/include -O2',
    CGO_LDFLAGS: '-L/stale/lib',
    PATH: '/tools', DYLD_LIBRARY_PATH: '/old', LD_LIBRARY_PATH: '/old',
  });
  const quote = value => JSON.stringify(value.replaceAll('\\', '/'));
  assert.equal(environment.CGO_CFLAGS, `${quote(`-I${path.join(root, 'include')}`)} -I/stale/include -O2`);
  assert.equal(environment.CGO_LDFLAGS, `${quote(`-L${path.join(root, 'lib')}`)} -L/stale/lib`);
  const loader = process.platform === 'win32' ? 'PATH'
    : process.platform === 'darwin' ? 'DYLD_LIBRARY_PATH' : 'LD_LIBRARY_PATH';
  assert.equal(environment[loader].split(path.delimiter)[0], path.join(root, 'lib'));
});
