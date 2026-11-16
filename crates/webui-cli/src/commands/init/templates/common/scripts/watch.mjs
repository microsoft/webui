// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { lstatSync, readdirSync, watch } from 'node:fs';
import path from 'node:path';

const trees = new Set(['src', 'server', 'scripts']);
const configs = new Set(['tsconfig.json', 'package.json']);
const ignored = new Set(['node_modules', 'target', 'build', 'bin', 'obj', '.git']);

export function watchSources() {
  const directories = new Map();
  let version = 0;
  let timer;
  let rescan = false;
  let waiter;
  let failure;
  let closed = false;
  function wake() {
    const resolve = waiter;
    waiter = undefined;
    resolve?.();
  }
  function fail(error) {
    if (closed) return;
    failure = error;
    wake();
  }
  function changed(directory, event, filename) {
    if (closed) return;
    const name = filename?.toString();
    if (name && (ignored.has(name) || (directory === '.' && !trees.has(name) && !configs.has(name)))) return;
    rescan ||= event === 'rename' || !name;
    clearTimeout(timer);
    timer = setTimeout(() => {
      try {
        if (rescan) reconcile();
        rescan = false;
        version++;
        wake();
      } catch (error) {
        fail(error);
      }
    }, 50);
  }
  function retire(directory) {
    directories.get(directory)?.watcher.close();
    directories.delete(directory);
  }
  function invalidated(directory, entry) {
    if (directory === '.') return false;
    try {
      const info = lstatSync(directory);
      return !info.isDirectory() || info.isSymbolicLink() || info.dev !== entry.dev || info.ino !== entry.ino;
    } catch (error) {
      if (['ENOENT', 'ENOTDIR'].includes(error.code)) return true;
      throw error;
    }
  }
  function watcherError(directory, entry, error) {
    if (closed || directories.get(directory) !== entry) return;
    try {
      if (error.code === 'EPERM' && invalidated(directory, entry)) {
        retire(directory);
        changed(directory, 'rename', null);
        return;
      }
    } catch (inspectionError) {
      fail(inspectionError);
      return;
    }
    fail(error);
  }
  function reconcile() {
    const seen = new Set();
    const pending = ['.'];
    while (pending.length) {
      const directory = pending.pop();
      let entry;
      try {
        const info = lstatSync(directory);
        if (!info.isDirectory() || info.isSymbolicLink()) continue;
        entry = directories.get(directory);
        if (!entry || entry.dev !== info.dev || entry.ino !== info.ino) {
          retire(directory);
          entry = { dev: info.dev, ino: info.ino };
          const current = entry;
          entry.watcher = watch(directory, (event, name) => {
            if (directories.get(directory) === current) changed(directory, event, name);
          });
          entry.watcher.on('error', error => watcherError(directory, current, error));
          directories.set(directory, entry);
        }
        for (const child of readdirSync(directory, { withFileTypes: true })) {
          if (!child.isDirectory() || child.isSymbolicLink() || ignored.has(child.name)) continue;
          if (directory === '.' && !trees.has(child.name)) continue;
          pending.push(path.join(directory, child.name));
        }
        seen.add(directory);
      } catch (error) {
        const missing = ['ENOENT', 'ENOTDIR'].includes(error.code);
        if (directory !== '.' && (missing || (error.code === 'EPERM' && entry && invalidated(directory, entry)))) {
          retire(directory);
          continue;
        }
        throw error;
      }
    }
    for (const [directory, entry] of directories) {
      if (seen.has(directory)) continue;
      entry.watcher.close();
      directories.delete(directory);
    }
  }
  function close() {
    closed = true;
    clearTimeout(timer);
    for (const entry of directories.values()) entry.watcher.close();
    directories.clear();
    wake();
  }
  try {
    reconcile();
  } catch (error) {
    close();
    throw error;
  }
  return {
    get version() { return version; },
    async wait(previous) {
      if (!closed && !failure && previous === version) await new Promise(resolve => { waiter = resolve; });
      if (failure) throw failure;
    },
    close,
  };
}
