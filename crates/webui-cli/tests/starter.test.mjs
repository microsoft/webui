// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { once } from 'node:events';
import { cp, mkdir, mkdtemp, readFile, realpath, rename, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { test } from 'node:test';
import { hostCommands } from '../src/commands/init/templates/common/scripts/app.mjs';
import { requestPath, pageState } from '../src/commands/init/templates/common/scripts/http.mjs';
import { running, shutdown, spawnFixture, terminateTree } from './starter-process.mjs';

const common = fileURLToPath(new URL('../src/commands/init/templates/common/', import.meta.url));
const processes = new Map();

async function fixture(t, server = 'node') {
  const root = await mkdtemp(path.join(tmpdir(), 'webui init scripts '));
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
      await rm(root, { force: true, recursive: true });
    }
  });
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

async function symlinkFixtures(t, root) {
  // The child resolves relative paths from its real cwd, not the temp-dir alias.
  const publicDir = path.join(await realpath(root), 'dist/public');
  const synthetic = new Set([path.join(publicDir, 'permission-link.js')]);
  await writeFile(path.join(publicDir, 'permission-link.js'), 'must not be served');
  try {
    await symlink('../protocol.bin', path.join(publicDir, 'leak.js'), 'file');
  } catch (error) {
    if (process.platform !== 'win32' || !['EPERM', 'EACCES'].includes(error.code)) throw error;
    t.diagnostic('File symlinks require Windows privileges; exercising the lstat symlink boundary with an injected file link.');
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

async function waitFor(probe) {
  for (let attempt = 0; attempt < 100; attempt++) {
    try { if (await probe()) return; } catch {}
    await sleep(50);
  }
  assert.fail('Timed out waiting for fixture');
}

function launch(t, root, script, args = [], env = {}) {
  const child = spawnFixture(root, script, args, env);
  let output = '';
  child.stdout.on('data', data => { output += data; });
  child.stderr.on('data', data => { output += data; });
  const ended = once(child, 'exit');
  processes.get(root).push({ child, ended });
  return { child, ended, output: () => output };
}

test('all seven hosts have build commands and runtime-only async start commands', async () => {
  for (const host of ['rust', 'dotnet', 'c', 'cpp', 'go', 'node', 'bun']) {
    const commands = hostCommands(host);
    assert.ok(Array.isArray(commands.build));
    assert.equal(typeof commands.start, 'function');
    if (host !== 'rust') {
      const start = await commands.start();
      assert.ok(Array.isArray(start[1]));
      assert.ok(!JSON.stringify(start).includes('build.mjs'));
      assert.ok(!JSON.stringify(start).includes('sdk.mjs'));
    }
  }
  assert.throws(() => hostCommands('unknown'), /Unknown server/);
});

test('request paths and per-page states reject document and traversal ambiguity', () => {
  for (const target of ['/assets/%2e%2e/protocol.bin', '/assets/a\\b.js', '/%00', '/%XX', 'http://example.test/']) {
    assert.equal(requestPath(target), null);
  }
  assert.equal(requestPath('/about?query=1'), '/about');
  assert.deepEqual(pageState('/'), { title: 'Hello WebUI', count: 0, isHome: true });
  assert.deepEqual(pageState('/about'), { title: 'About WebUI', isHome: false });
  assert.equal(pageState('/missing'), null);
});

test('Rust build and start use Cargo artifacts with custom native targets and target directories', async t => {
  const root = await fixture(t, 'rust');
  await writeFile(path.join(root, 'scripts/build.mjs'), '');
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
  await writeFile(path.join(root, 'server/main.rs'), `
    fn main() {
      println!("FRESH_RUST_HOST");
      if std::env::var_os("FIXTURE_STAY").is_some() {
        loop { std::thread::sleep(std::time::Duration::from_secs(1)); }
      }
    }
  `);
  const native = execFileSync('rustc', ['-vV'], { encoding: 'utf8' }).split('\n')
    .find(line => line.startsWith('host: ')).slice(6).trim();
  for (const configuration of ['environment', 'config']) {
    const env = { CARGO_TARGET_DIR: path.join(root, 'custom target'), CARGO_BUILD_TARGET: native };
    if (configuration === 'config') {
      env.CARGO_BUILD_TARGET = undefined;
      await mkdir(path.join(root, '.cargo'));
      await writeFile(path.join(root, '.cargo/config.toml'), `[build]\ntarget = "${native}"\n`);
    }
    const build = launch(t, root, 'scripts/app.mjs', ['build'], env);
    const [built] = await build.ended;
    assert.equal(built, 0, build.output());
    const start = launch(t, root, 'scripts/app.mjs', ['start'], {
      ...env, CARGO_BUILD_TARGET: 'no-toolchain-needed-to-start', PATH: path.join(root, 'no-tools'),
    });
    const [started] = await start.ended;
    assert.equal(started, 0, start.output());
    assert.match(start.output(), /FRESH_RUST_HOST/);
    const development = launch(t, root, 'scripts/app.mjs', ['dev'], { ...env, FIXTURE_STAY: '1' });
    await waitFor(() => development.output().includes('FRESH_RUST_HOST'));
    await shutdown(development.child);
    assert.equal((await development.ended)[0], 0, development.output());
  }
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
    import { spawn } from 'node:child_process';
    const descendant = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { stdio: 'ignore' });
    console.log('HOST_READY:' + process.pid + ':' + descendant.pid);
    const timer = setInterval(() => {}, 1000);
    process.once('SIGTERM', () => { writeFileSync('stopped', 'yes'); clearInterval(timer); });
  `);
  const process = launch(t, root, 'scripts/app.mjs', ['start']);
  await waitFor(() => process.output().includes('HOST_READY'));
  const [, host, descendant] = process.output().match(/HOST_READY:(\d+):(\d+)/);
  await shutdown(process.child);
  const [code] = await process.ended;
  assert.equal(code, 0, process.output());
  await waitFor(() => !running(Number(host)) && !running(Number(descendant)));
  if (globalThis.process.platform !== 'win32') assert.equal(await readFile(path.join(root, 'stopped'), 'utf8'), 'yes');
});

test('fixture emergency cleanup terminates the complete child process tree', async t => {
  const root = await fixture(t);
  await writeFile(path.join(root, 'tree.mjs'), `
    import { spawn } from 'node:child_process';
    const child = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { stdio: 'ignore' });
    console.log('TREE:' + process.pid + ':' + child.pid);
    setInterval(() => {}, 1000);
  `);
  const tree = launch(t, root, 'tree.mjs');
  await waitFor(() => tree.output().includes('TREE:'));
  const pids = tree.output().match(/TREE:(\d+):(\d+)/).slice(1).map(Number);
  await terminateTree(tree.child);
  await tree.ended;
  await waitFor(() => pids.every(pid => !running(pid)));
});

test('terminal hangup stops the host and removes its shutdown handlers', { timeout: 15_000 }, async t => {
  const root = await fixture(t);
  await writeFile(path.join(root, 'server/main.mjs'), "console.log('HOST_READY:' + process.pid); setInterval(() => {}, 1000);");
  await writeFile(path.join(root, 'hangup.mjs'), `
    import assert from 'node:assert/strict';
    import { main } from './scripts/app.mjs';
    const before = ['SIGINT', 'SIGTERM', 'SIGHUP', 'message', 'disconnect'].map(name => process.listenerCount(name));
    const hangup = value => { if (value === 'hangup') process.emit('SIGHUP'); };
    process.once('message', hangup);
    await main('start');
    process.removeListener('message', hangup);
    assert.deepEqual(['SIGINT', 'SIGTERM', 'SIGHUP', 'message', 'disconnect'].map(name => process.listenerCount(name)), before);
    console.log('HANDLERS_REMOVED');
  `);
  const host = launch(t, root, 'hangup.mjs');
  await waitFor(() => host.output().includes('HOST_READY:'));
  const pid = Number(host.output().match(/HOST_READY:(\d+)/)[1]);
  t.after(() => terminateTree({ pid, exitCode: null, signalCode: null }));
  if (process.platform === 'win32') host.child.send('hangup');
  else host.child.kill('SIGHUP');
  const [code] = await host.ended;
  assert.equal(code, 0, host.output());
  assert.equal(running(pid), false);
  assert.match(host.output(), /HANDLERS_REMOVED/);
});

test('dev runs edited frontend build scripts in fresh processes', async t => {
  const root = await fixture(t);
  const builder = revision => `
    import assert from 'node:assert/strict';
    import { appendFileSync } from 'node:fs';
    import { pathToFileURL } from 'node:url';
    export async function buildFrontend(development) {
      assert.equal(development, true);
      appendFileSync('events', 'BUILD_${revision}:' + process.pid + '\\n');
    }
    if (import.meta.url === pathToFileURL(process.argv[1]).href) {
      assert.deepEqual(process.argv.slice(2), ['--development']);
      await buildFrontend(true);
    }
  `;
  await writeFile(path.join(root, 'scripts/build.mjs'), builder('FIRST'));
  await writeFile(path.join(root, 'server/main.mjs'), `
    import { appendFileSync } from 'node:fs';
    appendFileSync('events', 'HOST_START\\n');
    setInterval(() => {}, 1000);
  `);
  const host = launch(t, root, 'scripts/app.mjs', ['dev']);
  const events = () => readFile(path.join(root, 'events'), 'utf8');
  await waitFor(async () => (await events()).includes('HOST_START'));
  await writeFile(path.join(root, 'scripts/build.mjs'), builder('SECOND'));
  await waitFor(async () => (await events()).includes('BUILD_SECOND:') && (await events()).split('HOST_START').length === 3);
  const pids = [...(await events()).matchAll(/BUILD_[A-Z]+:(\d+)/g)].map(match => Number(match[1]));
  assert.equal(pids.length, 2);
  assert.notEqual(pids[0], pids[1]);
  assert.ok(pids.every(pid => !running(pid)));
  await shutdown(host.child);
  assert.equal((await host.ended)[0], 0, host.output());
});

test('native builds preserve ordinary tool output instead of parsing it as Cargo metadata', async t => {
  const root = await fixture(t, 'dotnet');
  await writeFile(path.join(root, 'scripts/build.mjs'), '');
  const loader = path.join(root, 'native-build-probe.mjs');
  await writeFile(loader, `
    import childProcess from 'node:child_process';
    import { syncBuiltinESMExports } from 'node:module';
    const original = childProcess.spawn;
    childProcess.spawn = (command, args, options) => command === 'dotnet'
      ? original(process.execPath, ['-e', 'console.log("Native build output is not JSON");'], options)
      : original(command, args, options);
    syncBuiltinESMExports();
  `);
  const build = launch(t, root, 'scripts/app.mjs', ['build'], {
    NODE_OPTIONS: `${process.env.NODE_OPTIONS ?? ''} --import ${JSON.stringify(pathToFileURL(loader).href)}`,
    WEBUI_SDK_DIR: path.join(root, 'missing-sdk'),
  });
  const [code] = await build.ended;
  assert.equal(code, 0, build.output());
  assert.match(build.output(), /Native build output is not JSON/);
});

test('.NET start uses restored assets without consulting a standalone SDK override', async t => {
  const root = await fixture(t, 'dotnet');
  const bin = path.join(root, 'tools');
  const output = path.join(root, 'server/bin/Release/net8.0');
  await mkdir(bin);
  await mkdir(output, { recursive: true });
  await symlink(process.execPath, path.join(bin, process.platform === 'win32' ? 'dotnet.exe' : 'dotnet'), 'file');
  await writeFile(path.join(output, 'package.json'), '{"type":"commonjs"}');
  await writeFile(path.join(output, 'App.dll'), "console.log('MANAGED_HOST_READY');");
  const managed = launch(t, root, 'scripts/app.mjs', ['start'], {
    PATH: `${bin}${path.delimiter}${process.env.PATH}`,
    WEBUI_SDK_DIR: path.join(root, 'missing-sdk'),
  });
  const [code] = await managed.ended;
  assert.equal(code, 0, managed.output());
  assert.match(managed.output(), /MANAGED_HOST_READY/);
});

test('dev coalesces edits, rebuilds before restarting the selected host, and recovers after failure', async t => {
  const root = await fixture(t);
  await writeFile(path.join(root, 'scripts/build.mjs'), `
    import assert from 'node:assert/strict';
    import { appendFile, readFile } from 'node:fs/promises';
    import { setTimeout } from 'node:timers/promises';
    {
      try {
        const pid = Number(await readFile('host-pid', 'utf8'));
        assert.throws(() => process.kill(pid, 0), { code: 'ESRCH' }, 'Previous host must stop before rebuilding');
      } catch (error) { if (error.code !== 'ENOENT') throw error; }
      await appendFile('events', 'BUILD_START\\n');
      await setTimeout(200);
      if ((await readFile('src/index.html', 'utf8')).includes('INVALID')) throw new Error('invalid fixture');
      await appendFile('events', 'BUILD_DONE\\n');
    }
  `);
  await writeFile(path.join(root, 'server/main.mjs'), `
    import { appendFileSync, writeFileSync } from 'node:fs';
    writeFileSync('host-pid', String(process.pid));
    appendFileSync('events', 'HOST_START\\n');
    const timer = setInterval(() => {}, 1000);
    process.once('SIGTERM', () => { appendFileSync('events', 'HOST_STOP\\n'); clearInterval(timer); });
  `);
  const child = launch(t, root, 'scripts/app.mjs', ['dev']);
  const events = () => readFile(path.join(root, 'events'), 'utf8');
  await waitFor(async () => (await events()).includes('HOST_START'));
  await writeFile(path.join(root, 'src/index.html'), 'INVALID');
  await waitFor(() => child.output().includes('Build failed'));
  assert.match(await events(), /HOST_START\n(?:HOST_STOP\n)?BUILD_START\n$/);
  await writeFile(path.join(root, 'src/index.html'), '<h1>Fixed</h1>');
  await waitFor(async () => (await events()).split('HOST_START').length === 3);
  await shutdown(child.child);
  const [code] = await child.ended;
  assert.equal(code, 0, child.output());
  assert.match(await events(), /BUILD_START\nBUILD_DONE\nHOST_START\n(?:HOST_STOP\n)?$/);
  assert.equal(running(Number(await readFile(path.join(root, 'host-pid'), 'utf8'))), false);
  if (process.platform !== 'win32') assert.match(await events(), /HOST_STOP\n$/);
});

test('dev watches nested and atomic saves, coalesces build-time edits, and ignores generated output', async t => {
  const root = await fixture(t);
  await writeFile(path.join(root, 'scripts/build.mjs'), `
    import { appendFile } from 'node:fs/promises';
    import { setTimeout } from 'node:timers/promises';
    {
      await appendFile('events', 'BUILD_START\\n');
      await setTimeout(200);
      await appendFile('events', 'BUILD_DONE\\n');
    }
  `);
  await writeFile(path.join(root, 'server/main.mjs'), `
    import { appendFileSync } from 'node:fs';
    appendFileSync('events', 'HOST_START\\n');
    const timer = setInterval(() => {}, 1000);
    process.once('SIGTERM', () => { appendFileSync('events', 'HOST_STOP\\n'); clearInterval(timer); });
  `);
  const process = launch(t, root, 'scripts/app.mjs', ['dev']);
  const events = () => readFile(path.join(root, 'events'), 'utf8');
  const starts = async () => (await events()).split('HOST_START').length - 1;
  await waitFor(async () => await starts() === 1);
  for (const directory of ['dist/public', '.webui/modules', 'server/target/debug', 'server/build', 'node_modules']) {
    await mkdir(path.join(root, directory), { recursive: true });
    await writeFile(path.join(root, directory, 'generated'), 'ignored');
  }
  await sleep(650);
  assert.equal(await starts(), 1);
  await writeFile(path.join(root, 'src/next.html'), 'atomic');
  await rename(path.join(root, 'src/next.html'), path.join(root, 'src/index.html'));
  await waitFor(async () => (await events()).endsWith('BUILD_START\n'));
  await mkdir(path.join(root, 'src/nested/deep'), { recursive: true });
  await writeFile(path.join(root, 'src/nested/deep/page.html'), 'during build');
  await waitFor(async () => (await events()).split('BUILD_DONE').length >= 4 && await starts() >= 2);
  await sleep(350);
  const before = await starts();
  await rename(path.join(root, 'src/nested'), path.join(root, 'src/renamed'));
  await waitFor(async () => await starts() > before);
  await sleep(350);
  const renamed = await starts();
  await writeFile(path.join(root, 'src/renamed/deep/page.html'), 'nested edit');
  await waitFor(async () => await starts() > renamed);
  for (const directory of ['src/build', 'src/bin', 'src/target', 'src/obj']) {
    await mkdir(path.join(root, directory), { recursive: true });
    let before = await starts();
    await waitFor(async () => await starts() > before);
    await sleep(350);
    before = await starts();
    await writeFile(path.join(root, directory, 'page.html'), 'ordinary source');
    await waitFor(async () => await starts() > before);
  }
  await writeFile(path.join(root, 'tsconfig.tmp'), '{}');
  await rename(path.join(root, 'tsconfig.tmp'), path.join(root, 'tsconfig.json'));
  const count = await starts();
  await waitFor(async () => await starts() > count);
  await shutdown(process.child);
  const [code] = await process.ended;
  assert.equal(code, 0, process.output());
  assert.doesNotMatch(await events(), /BUILD_START\nBUILD_START/);
});

test('dev performs no idle filesystem scans after startup', async t => {
  const root = await fixture(t);
  await writeFile(path.join(root, 'scripts/build.mjs'), '');
  await writeFile(path.join(root, 'server/main.mjs'), "console.log('HOST_READY'); setInterval(() => {}, 1000);");
  await writeFile(path.join(root, 'metrics.mjs'), `
    import fs from 'node:fs';
    import promises from 'node:fs/promises';
    import { syncBuiltinESMExports } from 'node:module';
    let calls = 0;
    for (const [object, names] of [[fs, ['lstatSync', 'readdirSync']], [promises, ['stat', 'readdir']]]) {
      for (const name of names) {
        const original = object[name];
        object[name] = (...args) => { calls++; return original(...args); };
      }
    }
    syncBuiltinESMExports();
    await import('./scripts/app.mjs').then(app => app.main('dev'));
  `);
  // The probe runs in the supervisor, not in the server child.
  const probe = await readFile(path.join(root, 'metrics.mjs'), 'utf8');
  await writeFile(path.join(root, 'metrics.mjs'), probe.replace("await import(", `
    setTimeout(() => {
      const before = calls;
      setTimeout(() => { console.log('IDLE_SCANS:' + (calls - before)); process.emit('SIGTERM'); }, 1100);
    }, 500);
    await import(`));
  const process = launch(t, root, 'metrics.mjs');
  const [code] = await process.ended;
  assert.equal(code, 0, process.output());
  assert.match(process.output(), /HOST_READY/);
  assert.match(process.output(), /IDLE_SCANS:0/);
});

test('native watcher excludes generated registrations, handles nameless events, and surfaces errors', async t => {
  const root = await fixture(t);
  for (const directory of ['server/target/deep', 'server/build/deep', 'server/bin/deep', 'server/obj/deep', 'src/nested',
    'src/build', 'src/bin', 'src/target', 'src/obj', 'src/node_modules/deep', 'src/.git/deep']) {
    await mkdir(path.join(root, directory), { recursive: true });
  }
  await writeFile(path.join(root, 'watch-probe.mjs'), `
    import assert from 'node:assert/strict';
    import fs from 'node:fs';
    import path from 'node:path';
    import { syncBuiltinESMExports } from 'node:module';
    const registered = [];
    const original = fs.watch;
    fs.watch = (...args) => {
      const watcher = original(...args);
      registered.push({ directory: args[0], watcher });
      return watcher;
    };
    syncBuiltinESMExports();
    const { watchSources } = await import('./scripts/watch.mjs');
    const sources = watchSources();
    try {
      assert.deepEqual(registered.map(entry => entry.directory.split(path.sep).join('/')).sort(),
        process.platform === 'win32' ? ['.'] :
          ['.', 'scripts', 'server', 'src', 'src/bin', 'src/build', 'src/nested', 'src/obj', 'src/target']);
      let ignoredVersion = sources.version;
      const server = registered.find(entry => entry.directory === (process.platform === 'win32' ? '.' : 'server'));
      server.watcher.emit('change', 'change', process.platform === 'win32' ? path.join('server', 'target') : 'target');
      await new Promise(resolve => setTimeout(resolve, 100));
      assert.equal(sources.version, ignoredVersion);
      const version = sources.version;
      registered[0].watcher.emit('change', 'rename', null);
      await sources.wait(version);
      assert.ok(sources.version > version);
      registered[0].watcher.emit('error', new Error('WATCH_FAILED'));
      await assert.rejects(sources.wait(sources.version), /WATCH_FAILED/);
      console.log('WATCH_CONTRACT_PASSED');
    } finally { sources.close(); }
  `);
  const child = launch(t, root, 'watch-probe.mjs');
  const [code] = await child.ended;
  assert.equal(code, 0, child.output());
  assert.match(child.output(), /WATCH_CONTRACT_PASSED/);
});

test('native watcher recovers deleted Windows handles and registration races without hiding permission failures', async t => {
  const root = await fixture(t);
  for (const directory of ['deleted', 'during-register', 'during-read', 'protected']) {
    await mkdir(path.join(root, 'src', directory));
  }
  await writeFile(path.join(root, 'watch-races.mjs'), `
    import assert from 'node:assert/strict';
    import fs from 'node:fs';
    import path from 'node:path';
    import { syncBuiltinESMExports } from 'node:module';
    // Exercise per-directory registration recovery even on recursive-watch platforms.
    Object.defineProperty(process, 'platform', { value: 'linux' });
    const registered = new Map();
    const originalWatch = fs.watch;
    const originalRead = fs.readdirSync;
    fs.watch = (directory, ...args) => {
      if (path.basename(directory) === 'during-register') {
        fs.rmSync(directory, { recursive: true });
        throw Object.assign(new Error('directory removed before watch'), { code: 'ENOENT' });
      }
      const watcher = originalWatch(directory, ...args);
      registered.set(directory, watcher);
      return watcher;
    };
    fs.readdirSync = (directory, ...args) => {
      if (path.basename(directory) === 'during-read') {
        fs.rmSync(directory, { recursive: true });
        throw Object.assign(new Error('directory removed before read'), { code: 'ENOTDIR' });
      }
      return originalRead(directory, ...args);
    };
    syncBuiltinESMExports();
    const { watchSources } = await import('./scripts/watch.mjs');
    const sources = watchSources();
    try {
      const directory = path.join('src', 'deleted');
      const stale = registered.get(directory);
      assert.ok(stale);
      let version = sources.version;
      fs.rmSync(directory, { recursive: true });
      stale.emit('error', Object.assign(new Error('Windows deleted directory'), { code: 'EPERM' }));
      await sources.wait(version);
      version = sources.version;
      fs.mkdirSync(directory);
      await sources.wait(version);
      const current = registered.get(directory);
      assert.notEqual(current, stale);
      stale.emit('error', new Error('retired handle'));
      version = sources.version;
      fs.writeFileSync(path.join(directory, 'restored.html'), 'restored');
      await sources.wait(version);
      const protectedWatcher = registered.get(path.join('src', 'protected'));
      protectedWatcher.emit('error', Object.assign(new Error('real permission failure'), { code: 'EPERM' }));
      await assert.rejects(sources.wait(sources.version), /real permission failure/);
      sources.close();
      current.emit('error', new Error('closed handle'));
      console.log('WATCH_RACES_PASSED');
    } finally { sources.close(); }
  `);
  const child = launch(t, root, 'watch-races.mjs');
  const [code] = await child.ended;
  assert.equal(code, 0, child.output());
  assert.match(child.output(), /WATCH_RACES_PASSED/);
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
  const linkEnvironment = await symlinkFixtures(t, root);
  // Ask the OS for an unused port, then hand it to the real host.
  const { createServer } = await import('node:net');
  const reservation = createServer().listen(0, '127.0.0.1');
  await once(reservation, 'listening');
  const port = reservation.address().port;
  await new Promise(resolve => reservation.close(resolve));
  const child = launch(t, root, 'server/main.mjs', [], { ...linkEnvironment, PORT: String(port), HOST: '127.0.0.1' });
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
  for (const url of ['/missing', '/protocol.bin', '/assets/private.bin', '/assets/leak.js', '/assets/permission-link.js',
    '/assets/link/app.css', '/assets/escape/private.js', '/assets/%2e%2e%2fprotocol.bin',
    '/assets/index.js/missing.css', `/assets/${'x'.repeat(256)}.css`]) {
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
  await terminateTree(child.child);
  await child.ended;
  const failing = launch(t, root, 'server/main.mjs', [], {
    PORT: String(port), HOST: '127.0.0.1', FIXTURE_FAIL_RENDER: '1',
  });
  await waitFor(() => failing.output().includes('Listening on'));
  const failure = await fetch(base + '/');
  assert.equal(failure.status, 500);
  assert.equal(await failure.text(), 'Internal server error');
  assert.match(failing.output(), /private render details/);
  await terminateTree(failing.child);
  await failing.ended;
  await writeFile(path.join(root, 'dist/protocol.bin'), 'broken');
  const invalid = launch(t, root, 'server/main.mjs', [], { PORT: String(port) });
  const [code] = await invalid.ended;
  assert.notEqual(code, 0);
  assert.match(invalid.output(), /Bad protocol/);
  assert.ok(!invalid.output().includes('Listening on'));
});
