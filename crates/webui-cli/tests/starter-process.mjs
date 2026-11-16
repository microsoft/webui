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
    child.send('shutdown');
    await Promise.race([
      stopped,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error('Supervisor shutdown timed out')), 10_000);
      }),
    ]);
  } finally {
    clearTimeout(timer);
    await terminateTree(child);
  }
}

export function spawnFixture(root, script, args = [], env = {}) {
  return spawn(process.execPath, [script, ...args], {
    cwd: root,
    env: { ...process.env, ...env },
    stdio: ['ignore', 'pipe', 'pipe', 'ipc'],
  });
}
