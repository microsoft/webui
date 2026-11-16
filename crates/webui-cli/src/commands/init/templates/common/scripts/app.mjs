// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { spawn } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import path from 'node:path';
import { createInterface } from 'node:readline';
import { pathToFileURL } from 'node:url';
import { watchSources } from './watch.mjs';

const server = '__SERVER__';
const executable = process.platform === 'win32' ? 'webui-app.exe' : 'webui-app';
const rustArtifact = '.webui/server-executable.json';

export function hostCommands(host = server) {
  switch (host) {
    case 'rust': {
      const target = path.resolve('server/target');
      return {
        build: [['cargo', ['build', '--release', '--manifest-path', 'server/Cargo.toml', '--target-dir', target, '--message-format=json-render-diagnostics']]],
        async start() {
          try {
            const artifact = JSON.parse(await readFile(rustArtifact, 'utf8'));
            if (typeof artifact.executable !== 'string' || !artifact.executable) throw new Error('Invalid executable path');
            return [path.resolve(artifact.executable), []];
          } catch (error) {
            throw new Error('No Rust server build found. Run the package build command before start.', { cause: error });
          }
        },
      };
    }
    case 'dotnet': return {
      build: [['dotnet', ['build', 'server/App.csproj', '--configuration', 'Release']]],
      async start() { return ['dotnet', ['server/bin/Release/net8.0/App.dll']]; },
    };
    case 'c':
    case 'cpp': return {
      build: [
        ['cmake', ['-S', 'server', '-B', 'server/build', '-DCMAKE_BUILD_TYPE=Release']],
        ['cmake', ['--build', 'server/build', '--config', 'Release']],
      ],
      async start() {
        return [path.resolve('server/build', ...(process.platform === 'win32' ? ['Release'] : []), executable), []];
      },
    };
    case 'go': return {
      build: [
        ['go', ['mod', 'download', 'all'], 'server'],
        ['go', ['build', '-o', path.resolve('server/build', executable), '.'], 'server'],
      ],
      async start() { return [path.resolve('server/build', executable), []]; },
    };
    case 'node': return { build: [], async start() { return [process.execPath, ['server/main.mjs']]; } };
    case 'bun': return { build: [], async start() { return ['bun', ['server/main.ts']]; } };
    default: throw new Error(`Unknown server: ${host}`);
  }
}

function launch([command, args, cwd = '.'], env, onLine) {
  const child = spawn(command, args, {
    cwd, env, stdio: onLine ? ['inherit', 'pipe', 'inherit'] : 'inherit', detached: process.platform !== 'win32',
  });
  if (onLine) createInterface({ input: child.stdout }).on('line', onLine);
  child.completion = new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('close', (code, signal) => resolve({ code, signal }));
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
  let environment = process.env;
  if (mode !== 'check' && ['c', 'cpp', 'go'].includes(server)) {
    const { provisionSdk, sdkEnvironment } = await import('./sdk.mjs');
    environment = sdkEnvironment(await provisionSdk());
  }
  if (server === 'go') environment = { ...environment, WEBUI_APP_ROOT: process.cwd() };
  let child;
  let sources;
  let quitting = false;
  const shutdown = () => { quitting = true; sources?.close(); void stop(child).catch(console.error); };
  const message = value => { if (value === 'shutdown') shutdown(); };
  process.once('SIGINT', shutdown);
  process.once('SIGTERM', shutdown);
  process.once('SIGHUP', shutdown);
  process.on('message', message);
  process.once('disconnect', shutdown);
  async function run(command, onLine) {
    if (quitting) return;
    child = launch(command, environment, onLine);
    const result = await child.completion;
    child = undefined;
    if (!quitting && result.code !== 0) throw new Error(`${command[0]} failed (${result.signal ?? result.code})`);
  }
  async function build(development) {
    await run([process.execPath, ['scripts/build.mjs', ...(development ? ['--development'] : [])]]);
    let executable;
    for (const command of commands.build) {
      await run(command, server === 'rust' ? line => {
        const artifact = JSON.parse(line);
        if (artifact.reason === 'compiler-artifact' && artifact.target.name === 'webui-app'
          && artifact.target.kind.includes('bin') && artifact.executable) executable = artifact.executable;
      } : undefined);
    }
    if (server === 'rust' && !quitting) {
      if (!executable) throw new Error('Cargo did not report the webui-app executable.');
      await mkdir('.webui', { recursive: true });
      await writeFile(rustArtifact, JSON.stringify({ executable: path.relative(process.cwd(), executable) }));
    }
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
      await run(await commands.start());
    } else {
      sources = watchSources();
      while (!quitting) {
        const version = sources.version;
        await stop(child);
        child = undefined;
        try {
          await build(true);
          if (!quitting && sources.version === version) {
            const command = await commands.start();
            if (!quitting && sources.version === version) child = launch(command, environment);
          }
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
    try { await stop(child); } finally {
      process.removeListener('SIGINT', shutdown);
      process.removeListener('SIGTERM', shutdown);
      process.removeListener('SIGHUP', shutdown);
      process.removeListener('message', message);
      process.removeListener('disconnect', shutdown);
      if (process.connected) process.disconnect();
    }
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  main(process.argv[2]).catch(error => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
