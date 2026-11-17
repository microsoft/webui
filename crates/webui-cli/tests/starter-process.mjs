// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { execFile, spawn } from 'node:child_process';
import { promisify } from 'node:util';

const execute = promisify(execFile);

export function running(pid) {
  try { process.kill(pid, 0); return true; } catch (error) {
    if (error.code === 'ESRCH') return false;
    throw error;
  }
}

export async function terminateTree(child) {
  if (child.exitCode !== null || child.signalCode !== null || !child.pid) return;
  if (process.platform === 'win32') {
    try { await execute('taskkill', ['/pid', String(child.pid), '/T', '/F']); } catch (error) {
      if (running(child.pid)) throw error;
    }
  } else {
    const { stdout } = await execute('ps', ['-axo', 'pid=,ppid=']);
    const descendants = new Set([child.pid]);
    const rows = stdout.trim().split('\n').map(line => line.trim().split(/\s+/).map(Number));
    let previous;
    do {
      previous = descendants.size;
      for (const [pid, parent] of rows) if (descendants.has(parent)) descendants.add(pid);
    } while (previous !== descendants.size);
    for (const pid of [...descendants].reverse()) {
      try { process.kill(pid, 'SIGKILL'); } catch (error) {
        if (error.code !== 'ESRCH') throw error;
      }
    }
  }
}

export async function shutdown(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const stopped = new Promise((resolve, reject) => {
    child.once('close', resolve);
    child.once('error', reject);
  });
  let timer;
  try {
    child.stdin.once('error', error => {
      if (error.code !== 'EPIPE') child.emit('error', error);
    });
    child.stdin.end('S');
    const code = await Promise.race([
      stopped,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error('Supervisor shutdown timed out')),
          16_000);
      }),
    ]);
    if (code !== 0 || child.signalCode) {
      throw new Error(`Native supervisor did not stop gracefully (${child.signalCode ?? code})`);
    }
  } finally {
    clearTimeout(timer);
    await terminateTree(child);
  }
}

export function spawnNativeFixture(root, cli, args = ['app', 'start'], env = {}) {
  return spawn(cli, [...args, '--control-stdin'], {
    cwd: root,
    env: { ...process.env, ...env },
    stdio: ['pipe', 'pipe', 'pipe'],
  });
}

export function spawnHostFixture(root, script, env = {}) {
  return spawn(process.execPath, [script], {
    cwd: root,
    env: { ...process.env, ...env, _WEBUI_APP_CONTROL: 'stdin-v1' },
    stdio: ['pipe', 'pipe', 'pipe'],
  });
}
