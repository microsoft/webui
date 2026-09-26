// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const workspace = path.resolve(root, '..', '..');
const version = JSON.parse(readFileSync(path.join(root, 'package.json'), 'utf8')).version;
const native = `${process.platform}-${process.arch}`;
const packLocally = process.argv[2] === '--pack';
const tarballs = packLocally ? mkdtempSync(path.join(tmpdir(), 'webui-desktop-tarballs-')) : process.argv[2];
assert(tarballs, 'pass a directory with packed core and desktop npm artifacts, or --pack');

function tarball(name) {
  const file = path.join(tarballs, `microsoft-${name}-${version}.tgz`);
  assert(existsSync(file), `missing packed npm artifact: ${file}`);
  return file;
}

function run(command, args, cwd, extraEnv = {}) {
  const { WEBUI_DESKTOP_BINARY: _override, ...env } = process.env;
  const result = spawnSync(command, args, {
    cwd,
    env: { ...env, ...extraEnv },
    encoding: 'utf8',
    shell: process.platform === 'win32' && (command === 'npm' || command === 'pnpm'),
    timeout: 120_000,
  });
  assert(!result.error, `${command} failed to launch: ${result.error}`);
  return result;
}

function packageBin(cwd, name, args) {
  const bin = path.join(cwd, 'node_modules', '.bin', process.platform === 'win32' ? `${name}.cmd` : name);
  assert(existsSync(bin), `missing installed CLI shim: ${bin}`);
  if (process.platform === 'win32') {
    const quoted = [bin, ...args].map(arg => `'${arg.replaceAll("'", "''")}'`).join(' ');
    return run('pwsh', ['-NoProfile', '-NonInteractive', '-Command', `& ${quoted}; exit $LASTEXITCODE`], cwd);
  }
  return run(bin, args, cwd);
}

function webui(cwd, args) {
  return packageBin(cwd, 'webui', args);
}

function success(result) {
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
}

if (packLocally) {
  for (const name of ['webui', `webui-${native}`, 'webui-desktop', `webui-desktop-${native}`]) {
    const result = run('pnpm', [
      '--dir', path.join(workspace, 'packages', name),
      'pack', '--pack-destination', tarballs,
    ], workspace);
    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
  }
}

test('fresh npm consumer requires opt-in desktop support and then uses its native sidecar', (t) => {
  if (packLocally) t.after(() => rmSync(tarballs, { recursive: true, force: true }));
  const project = mkdtempSync(path.join(tmpdir(), 'webui-desktop-consumer-'));
  t.after(() => rmSync(project, { recursive: true, force: true }));
  writeFileSync(path.join(project, 'package.json'), '{"private":true}');

  success(run('npm', [
    'install', '--offline', '--omit=optional', '--no-audit', '--no-fund',
    tarball('webui'), tarball(`webui-${native}`),
  ], project));
  const coreManifest = JSON.parse(readFileSync(path.join(project, 'node_modules/@microsoft/webui/package.json'), 'utf8'));
  assert(!Object.keys(coreManifest.optionalDependencies).some(name => name.startsWith('@microsoft/webui-desktop')));

  const missing = webui(project, ['desktop', 'init', './app']);
  assert.equal(missing.status, 66, `${missing.stdout}\n${missing.stderr}`);
  assert.match(missing.stderr, /Desktop sidecar backend not found/);
  assert.match(missing.stderr, /npm install @microsoft\/webui-desktop/);

  success(run('npm', [
    'install', '--offline', '--omit=optional', '--no-audit', '--no-fund',
    tarball('webui-desktop'),
  ], project));
  const noNative = webui(project, ['desktop', 'init', './app']);
  assert.notEqual(noNative.status, 0);
  assert.match(noNative.stderr, new RegExp(`Missing @microsoft/webui-desktop-${native}`));

  success(run('npm', [
    'install', '--offline', '--omit=optional', '--no-audit', '--no-fund',
    tarball(`webui-desktop-${native}`),
  ], project));
  const nativeManifestPath = path.join(project, 'node_modules', '@microsoft', `webui-desktop-${native}`, 'package.json');
  const nativeManifest = readFileSync(nativeManifestPath, 'utf8');
  const skewedManifest = nativeManifest.replace(`"version": "${version}"`, '"version": "0.0.0"');
  assert.notEqual(skewedManifest, nativeManifest);
  writeFileSync(nativeManifestPath, skewedManifest);
  const mismatched = webui(project, ['desktop', 'init', './app']);
  assert.notEqual(mismatched.status, 0);
  assert(mismatched.stderr.includes(`version 0.0.0, but @microsoft/webui-desktop is ${version}`));
  writeFileSync(nativeManifestPath, nativeManifest);

  const standalone = packageBin(project, 'webui-desktop', ['--webui-version']);
  success(standalone);
  assert.equal(standalone.stdout.trim(), version);

  success(webui(project, ['desktop', 'init', './app']));
  assert(existsSync(path.join(project, 'app', 'src', 'index.html')));

  success(run('npm', [
    'exec', '--offline', '--', 'webui', 'desktop', 'build', './app/src', '--out', './bundle',
  ], project));
  assert(existsSync(path.join(project, 'bundle', 'protocol.bin')));
  assert(existsSync(path.join(project, 'bundle', 'manifest.webui-desktop.json')));

  const target = { darwin: 'macos-app', linux: 'linux-portable', win32: 'windows-portable' }[process.platform];
  assert(target, `unsupported desktop consumer test platform: ${process.platform}`);
  success(webui(project, ['desktop', 'package', './bundle', '--target', target, '--out', './packages']));
  assert(existsSync(path.join(project, 'packages')));
});
