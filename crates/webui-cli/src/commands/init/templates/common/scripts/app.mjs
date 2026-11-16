// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { spawn } from 'node:child_process';
import { createRequire } from 'node:module';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { watchSources } from './watch.mjs';

const server = '__SERVER__';
const executable = process.platform === 'win32' ? 'webui-app.exe' : 'webui-app';

export function hostCommands(host = server) {
  switch (host) {
    case 'rust': {
      const target = path.resolve('server/target');
      return {
        build: [['cargo', ['build', '--release', '--manifest-path', 'server/Cargo.toml', '--target-dir', target]]],
        start: [path.join(target, 'release', executable), []],
      };
    }
    case 'node': return { build: [], start: [process.execPath, ['server/main.mjs']] };
    case 'bun': return { build: [], start: ['bun', ['server/main.ts']] };
    default: throw new Error(`Unknown server: ${host}`);
  }
}

function launch([command, args, cwd = '.'], env) {
  const child = spawn(command, args, {
    cwd, env, stdio: 'inherit', detached: process.platform !== 'win32',
  });
  child.completion = new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('exit', (code, signal) => resolve({ code, signal }));
  });
  child.started = new Promise((resolve, reject) => {
    child.once('spawn', resolve);
    child.once('error', reject);
  });
  // Attach a rejection handler immediately, including while dev is rebuilding.
  child.completion.catch(() => {});
  child.started.catch(() => {});
  return child;
}

function stop(child) {
  if (!child) return Promise.resolve();
  return child.stopping ??= terminate(child);
}

async function terminate(child) {
  await child.started;
  if (!child?.pid || child.exitCode !== null || child.signalCode !== null) return;
  if (process.platform === 'win32') {
    const taskkill = spawn('taskkill', ['/pid', String(child.pid), '/T', '/F'], { stdio: 'ignore' });
    await new Promise(resolve => { taskkill.once('exit', resolve); taskkill.once('error', resolve); });
  } else {
    try { process.kill(-child.pid, 'SIGTERM'); } catch (error) {
      if (error.code !== 'ESRCH') throw error;
    }
    let timer;
    await Promise.race([
      child.completion,
      new Promise(resolve => {
        timer = setTimeout(() => {
          try { process.kill(-child.pid, 'SIGKILL'); } catch (error) {
            if (error.code !== 'ESRCH') console.error(error);
          }
          resolve();
        }, 6000);
      }),
    ]).finally(() => clearTimeout(timer));
  }
  await child.completion;
}

export async function main(mode) {
  if (!['dev', 'build', 'start', 'check'].includes(mode)) {
    throw new Error('Usage: node scripts/app.mjs <dev|build|start|check>');
  }
  const commands = hostCommands();
  const environment = process.env;
  let child;
  let sources;
  let quitting = false;
  const shutdown = () => { quitting = true; sources?.close(); void stop(child).catch(console.error); };
  process.once('SIGINT', shutdown);
  process.once('SIGTERM', shutdown);
  async function run(command) {
    if (quitting) return;
    child = launch(command, environment);
    const result = await child.completion;
    child = undefined;
    if (!quitting && result.code !== 0) throw new Error(`${command[0]} failed (${result.signal ?? result.code})`);
  }
  async function build(development) {
    const { buildFrontend } = await import('./build.mjs');
    await buildFrontend(development);
    for (const command of commands.build) await run(command);
  }
  try {
    if (mode === 'check') {
      const require = createRequire(import.meta.url);
      const manifest = require.resolve('typescript/package.json');
      const compiler = path.resolve(path.dirname(manifest), require(manifest).bin.tsc);
      await run([process.execPath, [compiler, '--project', 'tsconfig.json']]);
    } else if (mode === 'build') {
      await build(false);
    } else if (mode === 'start') {
      await run(commands.start);
    } else {
      sources = watchSources();
      while (!quitting) {
        const version = sources.version;
        await stop(child);
        child = undefined;
        try {
          await build(true);
          if (!quitting && sources.version === version) child = launch(commands.start, environment);
        } catch (error) {
          if (!quitting) console.error(`Build failed; fix the source to retry.\n${error.message}`);
        }
        if (quitting) break;
        await Promise.race([
          sources.wait(version),
          ...(child ? [child.completion.then(() => {
            if (!quitting) throw new Error('Development server exited. Fix the error and run dev again.');
          })] : []),
        ]);
      }
    }
  } finally {
    sources?.close();
    await stop(child);
    process.removeListener('SIGINT', shutdown);
    process.removeListener('SIGTERM', shutdown);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  main(process.argv[2]).catch(error => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
