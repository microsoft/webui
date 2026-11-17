// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { constants } from 'node:fs';
import { lstat, open, realpath } from 'node:fs/promises';
import path from 'node:path';

const mime = new Map([
  ['.js', 'text/javascript; charset=utf-8'],
  ['.css', 'text/css; charset=utf-8'],
  ['.json', 'application/json'],
  ['.map', 'application/json'],
  ['.txt', 'text/plain; charset=utf-8'],
  ['.svg', 'image/svg+xml'],
  ['.png', 'image/png'],
  ['.jpg', 'image/jpeg'],
  ['.ico', 'image/x-icon'],
  ['.woff2', 'font/woff2'],
]);

export function requestPath(target) {
  try {
    const pathname = decodeURIComponent(target.split('?')[0]);
    if (!pathname.startsWith('/') || pathname.includes('\\') || pathname.includes('\0')) return null;
    if (pathname.split('/').some(part => part === '.' || part === '..')) return null;
    return pathname;
  } catch {
    return null;
  }
}

export function pageState(pathname) {
  if (pathname === '/') return { title: 'Hello WebUI', count: 0, isHome: true };
  if (pathname === '/about') return { title: 'About WebUI', isHome: false };
  return null;
}

export function listenAddress() {
  const port = Number(process.env.PORT ?? '3000');
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error('PORT must be an integer from 1 to 65535.');
  return { hostname: process.env.HOST ?? '127.0.0.1', port };
}

// Public files only. Reject symlinks in every component and only expose known
// browser asset formats; a protocol, config, or source file is never an asset.
// The app owns dist; do not allow untrusted users to modify it while serving.
export async function openAsset(pathname) {
  if (!pathname?.startsWith('/assets/')) return null;
  const parts = pathname.slice('/assets/'.length).split('/');
  if (parts.some(part => !part || part.startsWith('.') || part.includes('\\') || part.includes('\0'))) return null;
  const contentType = mime.get(path.extname(parts.at(-1)).toLowerCase());
  if (!contentType) return null;
  let handle;
  try {
    let file = process.cwd();
    for (const part of ['dist', 'public', ...parts]) {
      file = path.join(file, part);
      if ((await lstat(file)).isSymbolicLink()) return null;
    }
    const root = await realpath('dist/public');
    const canonical = await realpath(file);
    if (!canonical.startsWith(`${root}${path.sep}`)) return null;
    handle = await open(file, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
    const info = await handle.stat();
    if (!info.isFile()) { await handle.close(); return null; }
    return {
      handle,
      headers: {
        'content-type': contentType,
        'content-length': String(info.size),
        'x-content-type-options': 'nosniff',
        'cache-control': 'no-cache',
      },
    };
  } catch (error) {
    await handle?.close();
    if (['ENOENT', 'ENOTDIR', 'ELOOP', 'EINVAL', 'ENAMETOOLONG'].includes(error.code)) return null;
    throw error;
  }
}
