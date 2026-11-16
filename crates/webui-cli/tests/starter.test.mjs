// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { cp, mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';
import { hostCommands } from '../src/commands/init/templates/common/scripts/app.mjs';
import { requestPath, pageState } from '../src/commands/init/templates/common/scripts/http.mjs';

const common = fileURLToPath(new URL('../src/commands/init/templates/common/', import.meta.url));

async function fixture(t, server = 'node') {
  const root = await mkdtemp(path.join(tmpdir(), 'webui init scripts '));
  t.after(() => rm(root, { force: true, recursive: true }));
  await cp(path.join(common, 'scripts'), path.join(root, 'scripts'), { recursive: true });
  await mkdir(path.join(root, 'src'));
  await mkdir(path.join(root, 'server'));
  await writeFile(path.join(root, 'src/index.html'), '<h1>Hello</h1>');
  await writeFile(path.join(root, 'tsconfig.json'), '{}');
  await writeFile(path.join(root, 'package.json'), '{"type":"module"}');
  const app = await readFile(path.join(root, 'scripts/app.mjs'), 'utf8');
  await writeFile(path.join(root, 'scripts/app.mjs'), app.replaceAll('__SERVER__', server));
  return root;
}

async function waitFor(probe) {
  for (let attempt = 0; attempt < 100; attempt++) {
    try { if (await probe()) return; } catch {}
    await sleep(50);
  }
  assert.fail('Timed out waiting for fixture');
}

function launch(t, root, script, args = [], env = {}) {
  const child = spawn(process.execPath, [script, ...args], { cwd: root, env: { ...process.env, ...env } });
  let output = '';
  child.stdout.on('data', data => { output += data; });
  child.stderr.on('data', data => { output += data; });
  const ended = once(child, 'exit');
  t.after(async () => {
    if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM');
    await ended;
  });
  return { child, ended, output: () => output };
}

test('all three hosts have build/start arrays with no runtime template compiler', () => {
  for (const host of ['rust', 'node', 'bun']) {
    const commands = hostCommands(host);
    assert.ok(Array.isArray(commands.build));
    assert.ok(Array.isArray(commands.start[1]));
    assert.ok(!JSON.stringify(commands.start).includes('build.mjs'));
    assert.ok(!JSON.stringify(commands.start).includes('sdk.mjs'));
  }
  for (const host of ['dotnet', 'c', 'cpp', 'go', 'unknown']) {
    assert.throws(() => hostCommands(host), /Unknown server/);
  }
});

test('request paths and per-page states reject document and traversal ambiguity', () => {
  for (const target of ['/assets/%2e%2e/protocol.bin', '/assets/a\\b.js', '/%00', '/%XX', 'http://example.test/']) {
    assert.equal(requestPath(target), null);
  }
  assert.equal(requestPath('/about?query=1'), '/about');
  assert.deepEqual(pageState('/'), { title: 'Hello WebUI', count: 0, ready: false, isHome: true });
  assert.deepEqual(pageState('/about'), { title: 'About WebUI', isHome: false });
  assert.equal(pageState('/missing'), null);
});

test('Rust build and start agree when CARGO_TARGET_DIR selects a different directory', async t => {
  const root = await fixture(t, 'rust');
  await writeFile(path.join(root, 'scripts/build.mjs'), 'export async function buildFrontend() {}');
  await writeFile(path.join(root, 'server/Cargo.toml'), `
    [package]
    name = "webui-app"
    version = "0.1.0"
    edition = "2021"
    [[bin]]
    name = "webui-app"
    path = "main.rs"
    [workspace]
  `);
  await writeFile(path.join(root, 'server/main.rs'), 'fn main() { println!("FRESH_RUST_HOST"); }');
  const env = { CARGO_TARGET_DIR: path.join(root, 'custom target') };
  const build = launch(t, root, 'scripts/app.mjs', ['build'], env);
  const [built] = await build.ended;
  assert.equal(built, 0, build.output());
  const start = launch(t, root, 'scripts/app.mjs', ['start'], env);
  const [started] = await start.ended;
  assert.equal(started, 0, start.output());
  assert.match(start.output(), /FRESH_RUST_HOST/);
});

test('check resolves and launches the compiler declared by the package resolver', async t => {
  const root = await fixture(t);
  const typescript = path.join(root, '.yarn/typescript');
  await mkdir(typescript, { recursive: true });
  await writeFile(path.join(typescript, 'package.json'), '{"bin":{"tsc":"compiler.cjs"}}');
  await writeFile(path.join(typescript, 'compiler.cjs'), `
    require('node:assert/strict').deepEqual(process.argv.slice(2), ['--project', 'tsconfig.json']);
    console.log('RESOLVED_TYPESCRIPT_CHECK');
  `);
  const loader = path.join(root, '.pnp.cjs');
  await writeFile(loader, `
    const Module = require('node:module');
    const resolve = Module._resolveFilename;
    Module._resolveFilename = function(specifier, ...args) {
      if (specifier === 'typescript/package.json') return require('node:path').join(__dirname, '.yarn/typescript/package.json');
      return resolve.call(this, specifier, ...args);
    };
  `);
  const check = launch(t, root, 'scripts/app.mjs', ['check'], {
    NODE_OPTIONS: `${process.env.NODE_OPTIONS ?? ''} --require ${JSON.stringify(loader)}`,
  });
  const [checked] = await check.ended;
  assert.equal(checked, 0, check.output());
  assert.match(check.output(), /RESOLVED_TYPESCRIPT_CHECK/);
});

test('start works without compiler dependencies and orderly shutdown stops its child', async t => {
  const root = await fixture(t);
  await writeFile(path.join(root, 'server/main.mjs'), `
    import { writeFileSync } from 'node:fs';
    console.log('HOST_READY');
    const timer = setInterval(() => {}, 1000);
    process.once('SIGTERM', () => { writeFileSync('stopped', 'yes'); clearInterval(timer); });
  `);
  const process = launch(t, root, 'scripts/app.mjs', ['start']);
  await waitFor(() => process.output().includes('HOST_READY'));
  process.child.kill('SIGTERM');
  const [code] = await process.ended;
  assert.equal(code, 0, process.output());
  assert.equal(await readFile(path.join(root, 'stopped'), 'utf8'), 'yes');
});

test('dev coalesces edits, rebuilds before restarting the selected host, and recovers after failure', async t => {
  const root = await fixture(t);
  await writeFile(path.join(root, 'scripts/build.mjs'), `
    import { appendFile, readFile } from 'node:fs/promises';
    import { setTimeout } from 'node:timers/promises';
    export async function buildFrontend() {
      await appendFile('events', 'BUILD_START\\n');
      await setTimeout(200);
      if ((await readFile('src/index.html', 'utf8')).includes('INVALID')) throw new Error('invalid fixture');
      await appendFile('events', 'BUILD_DONE\\n');
    }
  `);
  await writeFile(path.join(root, 'server/main.mjs'), `
    import { appendFileSync } from 'node:fs';
    appendFileSync('events', 'HOST_START\\n');
    const timer = setInterval(() => {}, 1000);
    process.once('SIGTERM', () => { appendFileSync('events', 'HOST_STOP\\n'); clearInterval(timer); });
  `);
  const child = launch(t, root, 'scripts/app.mjs', ['dev']);
  const events = () => readFile(path.join(root, 'events'), 'utf8');
  await waitFor(async () => (await events()).includes('HOST_START'));
  await writeFile(path.join(root, 'src/index.html'), 'INVALID');
  await waitFor(() => child.output().includes('Build failed'));
  assert.match(await events(), /HOST_START\nHOST_STOP\nBUILD_START\n$/);
  await writeFile(path.join(root, 'src/index.html'), '<h1>Fixed</h1>');
  await waitFor(async () => (await events()).split('HOST_START').length === 3);
  child.child.kill('SIGTERM');
  const [code] = await child.ended;
  assert.equal(code, 0, child.output());
  assert.match(await events(), /BUILD_START\nBUILD_DONE\nHOST_START\nHOST_STOP\n$/);
});

test('Node host HTTP boundary: GET/HEAD/405, private files, traversal, and symlinks', async t => {
  const root = await fixture(t);
  await cp(path.join(common, '../node/server/main.mjs'), path.join(root, 'server/main.mjs'));
  await mkdir(path.join(root, 'node_modules/@microsoft/webui'), { recursive: true });
  await writeFile(path.join(root, 'node_modules/@microsoft/webui/package.json'), '{"type":"module","exports":"./index.js"}');
  await writeFile(path.join(root, 'node_modules/@microsoft/webui/index.js'), `
    import { appendFileSync } from 'node:fs';
    export class Protocol {
      constructor(bytes, options) {
        if (bytes.toString() !== 'protocol' || options.plugin !== 'webui') throw new Error('Bad protocol');
        appendFileSync('loads', '1');
      }
      render(state, options) {
        if (options.entry !== 'index.html') throw new Error('Bad entry');
        if (process.env.FIXTURE_FAIL_RENDER === '1') throw new Error('private render details');
        return Buffer.from('<h1>' + state.title + '</h1>');
      }
    }
  `);
  await mkdir(path.join(root, 'dist/public/nested'), { recursive: true });
  await writeFile(path.join(root, 'dist/protocol.bin'), 'protocol');
  await writeFile(path.join(root, 'dist/public/index.js'), 'export {};');
  await writeFile(path.join(root, 'dist/public/nested/app.css'), 'body {}');
  await writeFile(path.join(root, 'dist/public/private.bin'), 'secret');
  await symlink('../protocol.bin', path.join(root, 'dist/public/leak.js'));
  await symlink('nested', path.join(root, 'dist/public/link'));
  // Ask the OS for an unused port, then hand it to the real host.
  const { createServer } = await import('node:net');
  const reservation = createServer().listen(0, '127.0.0.1');
  await once(reservation, 'listening');
  const port = reservation.address().port;
  await new Promise(resolve => reservation.close(resolve));
  const child = launch(t, root, 'server/main.mjs', [], { PORT: String(port), HOST: '127.0.0.1' });
  await waitFor(() => child.output().includes('Listening on'));
  const base = `http://127.0.0.1:${port}`;
  for (const [url, title] of [['/', 'Hello WebUI'], ['/about', 'About WebUI']]) {
    const response = await fetch(base + url);
    assert.equal(response.status, 200);
    assert.match(await response.text(), new RegExp(title));
    const head = await fetch(base + url, { method: 'HEAD' });
    assert.equal(head.status, 200);
    assert.equal(await head.text(), '');
  }
  for (const url of ['/missing', '/protocol.bin', '/assets/private.bin', '/assets/leak.js', '/assets/link/app.css', '/assets/%2e%2e%2fprotocol.bin']) {
    assert.equal((await fetch(base + url)).status, 404, url);
  }
  const asset = await fetch(base + '/assets/nested/app.css');
  assert.equal(asset.status, 200);
  assert.match(asset.headers.get('content-type'), /text\/css/);
  assert.equal(await asset.text(), 'body {}');
  const headAsset = await fetch(base + '/assets/index.js', { method: 'HEAD' });
  assert.equal(headAsset.status, 200);
  assert.equal(await headAsset.text(), '');
  const post = await fetch(base + '/', { method: 'POST' });
  assert.equal(post.status, 405);
  assert.equal(post.headers.get('allow'), 'GET, HEAD');
  assert.equal(await readFile(path.join(root, 'loads'), 'utf8'), '1');
  child.child.kill('SIGTERM');
  await child.ended;
  const failing = launch(t, root, 'server/main.mjs', [], {
    PORT: String(port), HOST: '127.0.0.1', FIXTURE_FAIL_RENDER: '1',
  });
  await waitFor(() => failing.output().includes('Listening on'));
  const failure = await fetch(base + '/');
  assert.equal(failure.status, 500);
  assert.equal(await failure.text(), 'Internal server error');
  assert.match(failing.output(), /private render details/);
  failing.child.kill('SIGTERM');
  await failing.ended;
  await writeFile(path.join(root, 'dist/protocol.bin'), 'broken');
  const invalid = launch(t, root, 'server/main.mjs', [], { PORT: String(port) });
  const [code] = await invalid.ended;
  assert.notEqual(code, 0);
  assert.match(invalid.output(), /Bad protocol/);
  assert.ok(!invalid.output().includes('Listening on'));
});
