// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { expect } from '@playwright/test';
import type { Page } from '@playwright/test';
import { once } from 'node:events';
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';

export const docsDist = path.resolve(import.meta.dirname, '../../../docs/dist');

export async function openAppearance(page: Page): Promise<void> {
  const trigger = page.getByRole('button', { name: 'Appearance', exact: true });
  const group = page.getByRole('group', { name: 'Color mode' });
  await expect.poll(async () => await group.isVisible() ||
    (await trigger.isVisible() && await trigger.isEnabled())).toBe(true);
  if (!await group.isVisible()) {
    await trigger.click();
  }
}

export async function setColorMode(page: Page, mode: 'system' | 'light' | 'dark'): Promise<void> {
  await openAppearance(page);
  const name = mode[0].toUpperCase() + mode.slice(1);
  const control = page.getByRole('group', { name: 'Color mode' }).getByRole('button', { name, exact: true });
  await control.click();
  await expect(control).toHaveAttribute('aria-pressed', 'true');
}

const contentTypes: Record<string, string> = {
  '.html': 'text/html; charset=utf-8',
  '.css': 'text/css',
  '.js': 'text/javascript',
  '.json': 'application/json',
  '.svg': 'image/svg+xml',
  '.wasm': 'application/wasm',
};

function luminance(color: string): number {
  const channels = color.match(/\d+/g)?.slice(0, 3).map(Number);
  assert.ok(channels && channels.length === 3);
  const [red, green, blue] = channels.map(value => {
    const channel = value / 255;
    return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

export function contrastRatio(color: string, background: string): number {
  const a = luminance(color);
  const b = luminance(background);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

export async function startWebsite(root = docsDist, basePath = '/webui/', headers: Record<string, string> = {}) {
  const directory = path.resolve(root);
  assert.ok(basePath.startsWith('/') && basePath.endsWith('/'));
  assert.ok(fs.existsSync(path.join(directory, 'index.html')), 'Build the documentation before testing the website.');
  const server = http.createServer((request, response) => {
    const pathname = new URL(request.url ?? '/', 'http://localhost').pathname;
    if (!pathname.startsWith(basePath)) {
      response.writeHead(404).end('Not found');
      return;
    }
    let relative: string;
    try {
      relative = decodeURIComponent(pathname.slice(basePath.length));
    } catch {
      response.writeHead(400).end('Invalid URL encoding');
      return;
    }
    const target = path.resolve(directory, relative || '.');
    if (target !== directory && !target.startsWith(directory + path.sep)) {
      response.writeHead(403).end('Outside site');
      return;
    }
    const file = path.extname(target) ? target : path.join(target, 'index.html');
    if (!fs.existsSync(file) || !fs.statSync(file).isFile()) {
      response.writeHead(404).end('Not found');
      return;
    }
    response.writeHead(200, {
      'content-type': contentTypes[path.extname(file)] ?? 'application/octet-stream',
      'cache-control': 'no-store',
      ...headers,
    });
    fs.createReadStream(file).on('error', error => response.destroy(error)).pipe(response);
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const address = server.address();
  assert.ok(address && typeof address !== 'string');
  return {
    origin: `http://127.0.0.1:${address.port}`,
    close: () => new Promise<void>((resolve, reject) => {
      server.close(error => error ? reject(error) : resolve());
      server.closeAllConnections();
    }),
  };
}
