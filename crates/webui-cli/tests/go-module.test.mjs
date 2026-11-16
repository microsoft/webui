// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { copyFile, mkdir, mkdtemp, readFile, realpath, rm, stat, writeFile } from 'node:fs/promises';
import { createConnection, createServer } from 'node:net';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { test } from 'node:test';
import { pathToFileURL } from 'node:url';
import { hostCommands } from '../src/commands/init/templates/common/scripts/app.mjs';

async function run(command, args, cwd, env) {
  const child = spawn(command, args, { cwd, env, stdio: ['ignore', 'pipe', 'pipe'] });
  let output = '';
  child.stdout.on('data', chunk => { output += chunk; });
  child.stderr.on('data', chunk => { output += chunk; });
  const [code] = await once(child, 'close');
  assert.equal(code, 0, `${command} ${args.join(' ')} failed:\n${output}`);
  return output;
}

test('a fresh generated Go app imports the staged official module and runs without Node', async () => {
  for (const name of ['WEBUI_GO_TEST_CLI', 'WEBUI_GO_TEST_PROTOCOL', 'WEBUI_GO_TEST_PROXY', 'WEBUI_GO_TEST_LIBRARY']) {
    assert.ok(process.env[name], `Run cargo xtask test-go: ${name} is required`);
  }
  const root = await realpath(await mkdtemp(path.join(tmpdir(), 'webui-go-consumer-')));
  const previous = process.cwd();
  let env = {
    ...process.env,
    GOWORK: 'off',
    GOTOOLCHAIN: 'local',
    GOMODCACHE: path.join(root, 'module-cache'),
    GOPROXY: pathToFileURL(`${process.env.WEBUI_GO_TEST_PROXY}${path.sep}`).href,
    // This unpublished fixture exists only in this isolated, local file proxy.
    GOSUMDB: 'off',
  };
  let server, closed;
  try {
    const app = path.join(root, 'consumer-app');
    await run(process.env.WEBUI_GO_TEST_CLI, ['init', app, '--server', 'go', '--no-interactive'], root, env);
    const module = await readFile(path.join(app, 'server/go.mod'), 'utf8');
    const dependency = module.match(/^require (github\.com\/microsoft\/webui\/go\/v\d+) (v\S+)$/m);
    assert.ok(dependency, module);
    assert.ok(!module.includes('replace'), 'consumer must use the module artifact, not a local replacement');
    assert.ok((await readFile(path.join(app, 'server/main.go'), 'utf8')).includes(dependency[1]));
    await assert.rejects(stat(path.join(app, 'server/webui.go')), { code: 'ENOENT' });
    await assert.rejects(stat(path.join(app, 'server/go.sum')), { code: 'ENOENT' });

    const stale = path.join(root, 'stale SDK/include');
    await mkdir(stale, { recursive: true });
    await writeFile(path.join(stale, 'webui_ffi.h'), '#error Inherited headers must not shadow the selected SDK\n');
    const { provisionSdk, sdkEnvironment } = await import(pathToFileURL(path.join(app, 'scripts/sdk.mjs')));
    env = sdkEnvironment(await provisionSdk(app), {
      ...env,
      CGO_CFLAGS: `${JSON.stringify(`-I${stale.replaceAll('\\', '/')}`)} ${env.CGO_CFLAGS ?? ''}`,
      CGO_CPPFLAGS: `${JSON.stringify(`-I${stale.replaceAll('\\', '/')}`)} ${env.CGO_CPPFLAGS ?? ''}`,
    });

    await mkdir(path.join(app, 'dist/public'), { recursive: true });
    await copyFile(process.env.WEBUI_GO_TEST_PROTOCOL, path.join(app, 'dist/protocol.bin'));
    await copyFile(process.env.WEBUI_GO_TEST_PROTOCOL, path.join(app, 'dist/public/private.bin'));
    await mkdir(path.join(app, 'dist/public/nested'));
    // The full framework browser suite covers JS; this checks the real Go host.
    await copyFile(process.env.WEBUI_GO_TEST_PROTOCOL, path.join(app, 'dist/public/nested/source.ts'));
    process.chdir(app);
    const commands = hostCommands('go');
    for (const [command, arguments_, cwd = '.'] of commands.build) {
      await run(command, arguments_, cwd, env);
    }
    const sum = await readFile(path.join(app, 'server/go.sum'), 'utf8');
    assert.ok(sum.split('\n').some(line => line.startsWith(`${dependency[1]} ${dependency[2]} h1:`)), sum);
    await run('go', ['test', '-race', '-count=1', './...'], 'server', env);

    const reservation = createServer().listen(0, '127.0.0.1');
    await once(reservation, 'listening');
    const port = reservation.address().port;
    await new Promise(resolve => reservation.close(resolve));
    const origin = `http://127.0.0.1:${port}`;
    const [executable, args] = commands.start;
    server = spawn(executable, args, {
      cwd: app,
      env: {
        ...env,
        HOST: '127.0.0.1', PORT: String(port), WEBUI_APP_ROOT: app,
        // Windows needs the DLL search path; neither location contains Node.
        PATH: process.platform === 'win32' ? path.dirname(process.env.WEBUI_GO_TEST_LIBRARY) : '',
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let output = '';
    server.stdout.on('data', chunk => { output += chunk; });
    server.stderr.on('data', chunk => { output += chunk; });
    closed = once(server, 'close');
    let ready = false;
    for (let attempt = 0; attempt < 100; attempt += 1) {
      assert.equal(server.exitCode, null, output);
      try {
        const response = await fetch(origin, { signal: AbortSignal.timeout(200) });
        const html = await response.text();
        assert.equal(response.status, 200, html);
        assert.ok(html.includes('Hello WebUI'), html);
        ready = true;
        break;
      } catch (error) {
        if (!(error instanceof TypeError) && error.name !== 'TimeoutError') throw error;
      }
      await sleep(50);
    }
    assert.ok(ready, `Go consumer did not start:\n${output}`);
    const about = await fetch(`${origin}/about`);
    assert.equal(about.status, 200);
    assert.ok((await about.text()).includes('About WebUI'));
    for (const target of ['/missing', '/protocol.bin', '/assets/private.bin', '/assets/nested/source.ts']) {
      assert.equal((await fetch(origin + target)).status, 404, target);
    }
    const head = await fetch(origin, { method: 'HEAD' });
    assert.equal(head.status, 200);
    assert.equal(await head.text(), '');
    assert.equal((await fetch(origin, { method: 'POST' })).status, 405);
    server.kill('SIGTERM');
    const [code, signal] = await closed;
    if (process.platform === 'win32') {
      // Node uses TerminateProcess, not a Go signal handler, on Windows.
      // The generated Go suite separately verifies real cancellation/draining.
      assert.equal(code, null, output);
      assert.equal(signal, 'SIGTERM', output);
    } else {
      assert.equal(code, 0, output);
    }
    const probe = createConnection({ host: '127.0.0.1', port });
    probe.setTimeout(1000, () => probe.destroy(new Error('Listener close probe timed out')));
    try {
      await assert.rejects(once(probe, 'connect'), { code: 'ECONNREFUSED' });
    } finally {
      probe.destroy();
    }
  } finally {
    if (server && server.exitCode === null && server.signalCode === null) {
      server.kill('SIGTERM');
      const timer = setTimeout(() => server.kill('SIGKILL'), 10_000);
      await closed.finally(() => clearTimeout(timer));
    }
    process.chdir(previous);
    await run('go', ['clean', '-modcache'], previous, env);
    await rm(root, { recursive: true, force: true });
  }
});
