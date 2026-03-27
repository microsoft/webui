// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import { existsSync, readFileSync } from 'node:fs';
import { extname, resolve } from 'node:path';

export type FixtureContentTypes = Record<string, string>;

export interface FixtureRequestContext {
  req: IncomingMessage;
  res: ServerResponse;
  url: URL;
  send: (
    status: number,
    body: string | Buffer,
    type?: string,
  ) => void;
  serveStatic: (
    pathname: string,
    contentTypes?: FixtureContentTypes,
  ) => boolean;
}

export interface FixtureServerOptions {
  name: string;
  fixturesRoot: string;
  port: number;
  handleRequest: (context: FixtureRequestContext) => boolean | void;
}

export const DEFAULT_CONTENT_TYPES: FixtureContentTypes = {
  '.css': 'text/css; charset=utf-8',
  '.html': 'text/html; charset=utf-8',
  '.js': 'application/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
};

export function sendResponse(
  res: ServerResponse,
  status: number,
  body: string | Buffer,
  type = 'text/plain; charset=utf-8',
): void {
  res.writeHead(status, { 'Content-Type': type });
  res.end(body);
}

export function resolveFixturePath(fixturesRoot: string, pathname: string): string | null {
  const relativePath = pathname === '/' ? '' : pathname.slice(1);
  const filePath = resolve(fixturesRoot, relativePath);
  if (!filePath.startsWith(fixturesRoot)) {
    return null;
  }

  return filePath;
}

export function serveFixtureFile(
  fixturesRoot: string,
  pathname: string,
  res: ServerResponse,
  contentTypes: FixtureContentTypes = DEFAULT_CONTENT_TYPES,
): boolean {
  const filePath = resolveFixturePath(fixturesRoot, pathname);
  if (!filePath || !existsSync(filePath)) {
    return false;
  }

  const type = contentTypes[extname(filePath)] ?? 'application/octet-stream';
  sendResponse(res, 200, readFileSync(filePath), type);
  return true;
}

export function startFixtureServer({
  name,
  fixturesRoot,
  port,
  handleRequest,
}: FixtureServerOptions): void {
  createServer((req, res) => {
    if (!req.url) {
      sendResponse(res, 500, 'Missing request URL');
      return;
    }

    const url = new URL(req.url, 'http://127.0.0.1');
    const handled = handleRequest({
      req,
      res,
      url,
      send: (status, body, type) => sendResponse(res, status, body, type),
      serveStatic: (pathname, contentTypes) =>
        serveFixtureFile(fixturesRoot, pathname, res, contentTypes),
    });

    if (handled) {
      return;
    }

    sendResponse(res, 404, 'Not found');
  }).listen(port, '127.0.0.1', () => {
    console.log(`${name} fixtures ready on http://127.0.0.1:${port}`);
  });
}
