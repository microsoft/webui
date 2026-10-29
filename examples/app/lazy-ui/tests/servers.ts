// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { execFileSync, spawn } from 'node:child_process';
import { once } from 'node:events';
import { existsSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import type { AddressInfo } from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

/** The example directory, where `webui serve` resolves `./src` and `./dist`. */
export const exampleRoot = fileURLToPath(new URL('..', import.meta.url));

const CONTENT_TYPES: Readonly<Record<string, string>> = {
  '.css': 'text/css; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
};

/** A running HTTP server owned by a test. */
export interface TestServer {
  readonly origin: string;
  stop(): Promise<void>;
}

/** A CDN stand-in: serves a build output directory on its own origin with CORS. */
export interface AssetOrigin extends TestServer {
  /** Paths requested so far, in order. */
  readonly requests: readonly string[];
}

export async function startAssetOrigin(directory: string): Promise<AssetOrigin> {
  const root = path.resolve(directory);
  const requests: string[] = [];
  const server = createServer((request, response) => {
    const { pathname } = new URL(request.url ?? '/', 'http://localhost');
    requests.push(pathname);
    response.setHeader('Access-Control-Allow-Origin', '*');
    let file: string;
    try {
      file = path.resolve(root, `.${decodeURIComponent(pathname)}`);
    } catch {
      response.writeHead(400).end();
      return;
    }
    const type = CONTENT_TYPES[path.extname(file)];
    if (!type || !file.startsWith(root + path.sep)) {
      response.writeHead(404).end();
      return;
    }
    readFile(file).then(body => {
      response.writeHead(200, { 'Content-Type': type, 'Cache-Control': 'no-cache' }).end(body);
    }, () => {
      response.writeHead(404).end();
    });
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  return {
    origin: `http://127.0.0.1:${(server.address() as AddressInfo).port}`,
    requests,
    stop: () => new Promise<void>(resolve => {
      server.close(() => resolve());
      server.closeAllConnections();
    }),
  };
}

/**
 * Serve the example with `webui serve` and a state file, without the sample
 * API, on a free port. Arguments match the example's `build:protocol` script,
 * so the pages reference the same packs and stylesheets as `dist/`.
 */
export async function startExampleServer(stateFile: string): Promise<TestServer> {
  const binary = webuiBinary();
  if (!existsSync(binary)) {
    throw new Error(`${binary} does not exist. Run \`pnpm run build\` in the example first.`);
  }
  const port = await freePort();
  const output: string[] = [];
  const child = spawn(binary, [
    'serve', './src',
    '--state', stateFile,
    '--plugin=webui',
    '--projection-manifest', './dist/webui-projection.json',
    '--theme=@microsoft/webui-examples-theme',
    '--servedir', './dist',
    '--port', String(port),
  ], { cwd: exampleRoot, stdio: ['ignore', 'pipe', 'pipe'] });
  let failure: Error | undefined;
  child.once('error', error => { failure = error; });
  child.stdout.on('data', (chunk: Buffer) => output.push(chunk.toString()));
  child.stderr.on('data', (chunk: Buffer) => output.push(chunk.toString()));
  const stop = async (): Promise<void> => {
    if (failure || child.exitCode !== null || child.signalCode !== null) return;
    const exited = once(child, 'exit');
    child.kill();
    await exited;
  };
  const origin = `http://127.0.0.1:${port}`;
  const deadline = Date.now() + 60_000;
  while (Date.now() < deadline && !failure && child.exitCode === null) {
    try {
      if ((await fetch(`${origin}/`)).ok) return { origin, stop };
    } catch {
      // Still building.
    }
    await new Promise(resolve => setTimeout(resolve, 200));
  }
  await stop();
  throw new Error(`webui serve did not start on ${origin}: ${failure?.message ?? ''}\n${output.join('')}`);
}

/** The debug CLI that the example's `cargo run` scripts build, wherever Cargo puts its target directory. */
function webuiBinary(): string {
  const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--format-version', '1', '--no-deps'], {
    cwd: exampleRoot,
    encoding: 'utf8',
    maxBuffer: 64 * 1024 * 1024,
  })) as { target_directory: string };
  return path.join(metadata.target_directory, 'debug', process.platform === 'win32' ? 'webui.exe' : 'webui');
}

async function freePort(): Promise<number> {
  const server = createServer();
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const { port } = server.address() as AddressInfo;
  await new Promise<void>(resolve => server.close(() => resolve()));
  return port;
}
