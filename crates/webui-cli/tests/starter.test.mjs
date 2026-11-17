// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { once } from 'node:events';
import { mkdir, mkdtemp, readFile, realpath, rename, rm, symlink, writeFile } from 'node:fs/promises';
import { get as httpGet } from 'node:http';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { test } from 'node:test';
import { promisify } from 'node:util';
import { requestPath, pageState } from '../src/commands/init/templates/runtime/server/http.mjs';
import { running, shutdown, spawnHostFixture, spawnNativeFixture, terminateTree } from './starter-process.mjs';

const execute = promisify(execFile);
const workspace = fileURLToPath(new URL('../../../', import.meta.url));
const target = path.resolve(process.env.CARGO_TARGET_DIR ?? path.join(workspace, 'target'), 'debug');
const cli = process.env.WEBUI_INIT_TEST_CLI ?? path.join(target, process.platform === 'win32' ? 'webui.exe' : 'webui');
const addon = process.env.WEBUI_ADDON_PATH ?? path.join(target, process.platform === 'win32'
  ? 'webui_node.dll' : process.platform === 'darwin' ? 'libwebui_node.dylib' : 'libwebui_node.so');
const processes = new Map();

async function fixture(t) {
  // Init rejects symlink ancestors; macOS /var and /private/var name the same temp directory.
  const root = path.join(await realpath(await mkdtemp(path.join(tmpdir(), 'webui native starter '))), 'app');
  const children = [];
  processes.set(root, children);
  t.after(async () => {
    try {
      for (const { child, ended } of children) {
        await terminateTree(child);
        await ended;
      }
    } finally {
      processes.delete(root);
      await rm(path.dirname(root), { force: true, recursive: true });
    }
  });
  await execute(cli, ['init', root, '--server', 'node', '--no-interactive'], {
    env: { ...process.env, PATH: '' },
  });
  await mkdir(path.join(root, 'node_modules/@microsoft'), { recursive: true });
  await symlink(path.join(workspace, 'packages/webui'), path.join(root, 'node_modules/@microsoft/webui'),
    process.platform === 'win32' ? 'junction' : 'dir');
  await execute(cli, ['app', 'build', '--project', root], {
    env: { ...process.env, PATH: '' },
  });
  return root;
}

async function port() {
  const reservation = createServer().listen(0, '127.0.0.1');
  await once(reservation, 'listening');
  const port = reservation.address().port;
  await new Promise(resolve => reservation.close(resolve));
  return port;
}

async function waitFor(probe, output = () => '') {
  for (let attempt = 0; attempt < 200; attempt++) {
    if (await probe()) return;
    await sleep(50);
  }
  assert.fail(`Timed out waiting for fixture:\n${output()}`);
}

function launch(root, env, action = 'start', direct = false) {
  const environment = { WEBUI_ADDON_PATH: addon, HOST: '127.0.0.1', ...env };
  const child = direct ? spawnHostFixture(root, 'server/main.mjs', environment)
    : spawnNativeFixture(root, cli, ['app', action], environment);
  let output = '';
  child.stdout.on('data', data => { output += data; });
  child.stderr.on('data', data => { output += data; });
  const ended = once(child, 'close');
  processes.get(root).push({ child, ended });
  return { child, ended, output: () => output };
}

async function symlinkFixtures(t, root) {
  const publicDir = path.join(root, 'dist/public');
  const synthetic = new Set([path.join(publicDir, 'permission-link.js')]);
  await writeFile(path.join(publicDir, 'permission-link.js'), 'must not be served');
  try {
    await symlink('../protocol.bin', path.join(publicDir, 'leak.js'), 'file');
  } catch (error) {
    if (process.platform !== 'win32' || !['EPERM', 'EACCES'].includes(error.code)) throw error;
    t.diagnostic('File symlinks require Windows privileges; injecting lstat file-link identity.');
    await writeFile(path.join(publicDir, 'leak.js'), 'must not be served');
    synthetic.add(path.join(publicDir, 'leak.js'));
  }
  const kind = process.platform === 'win32' ? 'junction' : 'dir';
  await symlink(path.join(publicDir, 'nested'), path.join(publicDir, 'link'), kind);
  await mkdir(path.join(root, 'private-assets'));
  await writeFile(path.join(root, 'private-assets/private.js'), 'private');
  await symlink(path.join(root, 'private-assets'), path.join(publicDir, 'escape'), kind);
  const loader = path.join(root, 'symlink-probe.mjs');
  await writeFile(loader, `
    import fs from 'node:fs/promises';
    import path from 'node:path';
    import { syncBuiltinESMExports } from 'node:module';
    const synthetic = new Set(${JSON.stringify([...synthetic])});
    const original = fs.lstat;
    fs.lstat = async (file, ...args) => {
      const info = await original(file, ...args);
      if (synthetic.has(path.resolve(file))) info.isSymbolicLink = () => true;
      return info;
    };
    syncBuiltinESMExports();
  `);
  return { NODE_OPTIONS: `${process.env.NODE_OPTIONS ?? ''} --import ${JSON.stringify(pathToFileURL(loader).href)}` };
}

test('request paths and per-page states reject document and traversal ambiguity', () => {
  for (const target of ['/assets/%2e%2e/protocol.bin', '/assets/a\\b.js', '/%00', '/%XX', 'http://example.test/']) {
    assert.equal(requestPath(target), null);
  }
  assert.equal(requestPath('/about?query=1'), '/about');
  assert.deepEqual(pageState('/'), { title: 'Hello WebUI', count: 0, isHome: true });
  assert.deepEqual(pageState('/about'), { title: 'About WebUI', isHome: false });
  assert.equal(pageState('/missing'), null);
});

test('real Node renderer HTTP boundary: GET/HEAD/405, private files, traversal, symlinks and failures', async t => {
  const root = await fixture(t);
  await mkdir(path.join(root, 'dist/public/nested'));
  await writeFile(path.join(root, 'dist/public/nested/app.css'), 'body {}');
  await writeFile(path.join(root, 'dist/public/private.bin'), 'secret');
  const linkEnvironment = await symlinkFixtures(t, root);
  const listener = await port();
  const host = launch(root, { ...linkEnvironment, PORT: String(listener) });
  await waitFor(() => host.output().includes('Listening on'), host.output);
  // A loaded protocol must not be reopened on subsequent requests.
  await rename(path.join(root, 'dist/protocol.bin'), path.join(root, 'dist/held.bin'));
  const base = `http://127.0.0.1:${listener}`;
  for (const [url, title] of [['/', 'Hello WebUI'], ['/about', 'About WebUI']]) {
    const response = await fetch(base + url);
    assert.equal(response.status, 200);
    assert.ok((await response.text()).includes(title));
    const head = await fetch(base + url, { method: 'HEAD' });
    assert.equal(head.status, 200);
    assert.equal(await head.text(), '');
  }
  for (const url of ['/missing', '/protocol.bin', '/assets/private.bin', '/assets/leak.js', '/assets/permission-link.js',
    '/assets/link/app.css', '/assets/escape/private.js', '/assets/%2e%2e%2fprotocol.bin',
    '/assets/index.js/missing.css', `/assets/${'x'.repeat(256)}.css`]) {
    for (const method of ['GET', 'HEAD']) assert.equal((await fetch(base + url, { method })).status, 404, url);
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
  await shutdown(host.child);
  await host.ended;
  await rename(path.join(root, 'dist/held.bin'), path.join(root, 'dist/protocol.bin'));

  // Force the real renderer's entry-not-found failure through the unmodified HTTP catch boundary.
  const source = path.join(root, 'server/main.mjs');
  await writeFile(source, (await readFile(source, 'utf8')).replace("entry: 'index.html'", "entry: 'missing.html'"));
  const failing = launch(root, { PORT: String(listener) }, 'start', true);
  await waitFor(() => failing.output().includes('Listening on'), failing.output);
  const failure = await fetch(base + '/');
  assert.equal(failure.status, 500);
  assert.equal(await failure.text(), 'Internal server error');
  assert.match(failing.output(), /Request failed/);
  await shutdown(failing.child);
  await failing.ended;

  await writeFile(path.join(root, 'dist/protocol.bin'), 'broken');
  const invalid = launch(root, { PORT: String(listener) }, 'start', true);
  assert.notEqual((await invalid.ended)[0], 0);
  assert.ok(!invalid.output().includes('Listening on'), invalid.output());
});

test('native start drains a backpressured HTTP response before releasing its process tree', async t => {
  const root = await fixture(t);
  const bytes = 12 * 1024 * 1024;
  await writeFile(path.join(root, 'dist/public/drain.css'), Buffer.alloc(bytes, 32));
  const listener = await port();
  const host = launch(root, { PORT: String(listener) });
  await waitFor(() => host.output().includes('Listening on'), host.output);
  const response = await new Promise((resolve, reject) => {
    httpGet(`http://127.0.0.1:${listener}/assets/drain.css`, { agent: false }, resolve).once('error', reject);
  });
  let stopping;
  try {
    assert.equal(response.statusCode, 200);
    response.pause();
    await sleep(100);
    let received = 0;
    const body = new Promise((resolve, reject) => {
      response.on('data', chunk => { received += chunk.length; });
      response.once('end', resolve);
      response.once('error', reject);
    });
    response.pause();
    stopping = shutdown(host.child);
    const completed = Promise.all([body, stopping]);
    await Promise.race([completed, sleep(100)]);
    assert.equal(host.child.exitCode, null, host.output());
    response.resume();
    await completed;
    assert.equal(received, bytes);
    assert.equal(response.complete, true);
  } finally {
    response.destroy();
    await stopping;
  }
  assert.equal((await host.ended)[0], 0, host.output());
});

test('native development rebuilds atomic/nested edits, ignores outputs, and recovers after failure', async t => {
  const root = await fixture(t);
  const listener = await port();
  const host = launch(root, { PORT: String(listener) }, 'dev');
  const starts = () => host.output().split('Listening on').length - 1;
  await waitFor(() => starts() === 1, host.output);
  const home = path.join(root, 'web/home-page/home-page.html');
  const original = await readFile(home, 'utf8');
  const next = path.join(root, 'web/home-page/atomic.html');
  await writeFile(next, original.replace('Make it yours:', 'Atomic native edit:'));
  await rename(next, home);
  await waitFor(() => starts() === 2, host.output);
  assert.match(await (await fetch(`http://127.0.0.1:${listener}`)).text(), /Atomic native edit:/);
  const nested = path.join(root, 'web/home-page/nested/deep');
  await mkdir(nested, { recursive: true });
  await writeFile(path.join(nested, 'note.txt'), 'new nested directory');
  await waitFor(() => starts() >= 3, host.output);
  const count = starts();
  await writeFile(path.join(nested, 'note.txt'), 'nested edit');
  await waitFor(() => starts() > count, host.output);
  await sleep(200);
  const before = starts();
  for (const directory of ['dist/public', '.webui', 'target', 'bin', 'obj', 'build']) {
    await mkdir(path.join(root, directory), { recursive: true });
    await writeFile(path.join(root, directory, 'ignored.txt'), 'generated');
  }
  await sleep(400);
  assert.equal(starts(), before, host.output());
  const component = path.join(root, 'web/app-counter/app-counter.ts');
  const script = await readFile(component, 'utf8');
  await writeFile(component, 'export class {');
  await waitFor(() => host.output().includes('error'), host.output);
  const failed = starts();
  await assert.rejects(fetch(`http://127.0.0.1:${listener}`, { signal: AbortSignal.timeout(500) }));
  await sleep(300);
  assert.equal(starts(), failed, 'failed builds must wait for a source edit');
  await writeFile(component, script);
  await waitFor(() => starts() > failed, host.output);
  await shutdown(host.child);
  assert.equal((await host.ended)[0], 0, host.output());
});

test('native supervisor treats control EOF and terminal hangup as graceful shutdown', async t => {
  const root = await fixture(t);
  for (const signal of ['eof', ...(process.platform === 'win32' ? [] : ['SIGHUP'])]) {
    const host = launch(root, { PORT: String(await port()) });
    await waitFor(() => host.output().includes('Listening on'), host.output);
    if (signal === 'eof') host.child.stdin.end();
    else host.child.kill(signal);
    assert.equal((await host.ended)[0], 0, host.output());
  }
});

test('fixture emergency cleanup terminates the complete child process tree', async t => {
  const root = await fixture(t);
  await writeFile(path.join(root, 'tree.mjs'), `
    import { spawn } from 'node:child_process';
    const child = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { stdio: 'ignore' });
    console.log('TREE:' + process.pid + ':' + child.pid);
    setInterval(() => {}, 1000);
  `);
  const child = spawnHostFixture(root, 'tree.mjs');
  const ended = once(child, 'close');
  processes.get(root).push({ child, ended });
  let output = '';
  child.stdout.on('data', data => { output += data; });
  await waitFor(() => output.includes('TREE:'));
  const pids = output.match(/TREE:(\d+):(\d+)/).slice(1).map(Number);
  await terminateTree(child);
  await ended;
  await waitFor(() => pids.every(pid => !running(pid)));
});
