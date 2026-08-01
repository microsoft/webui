// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';

import { STREAMING_STATE } from './data.js';
import { streamPage, WEATHER_DELAY_MIN_MS } from './pacing.js';
import {
  acceptsWebUIStream,
  WEBUI_STREAM_MEDIA_TYPE,
  WebUIStreamWriter,
} from './stream-protocol.js';
import { TestControls, type TestSession } from './test-controls.js';

interface ServerOptions {
  port: number;
  feedDelayMinMs: number;
  feedDelayMaxMs: number;
  maxConcurrentStreams: number;
  testControls: boolean;
}

const options = parseOptions(process.argv.slice(2));
const controls = options.testControls ? new TestControls() : undefined;
let activeStreams = 0;

const server = createServer((request, response) => {
  void handleRequest(request, response).catch((error: unknown) => {
    failRequest(response, error);
  });
});

server.listen(options.port, '127.0.0.1', () => {
  console.log(`Streaming API listening on http://127.0.0.1:${options.port}`);
});

async function handleRequest(
  request: IncomingMessage,
  response: ServerResponse,
): Promise<void> {
  const url = new URL(request.url ?? '/', 'http://127.0.0.1');
  if (request.method === 'GET' && url.pathname === '/health') {
    response.statusCode = 204;
    response.end();
    return;
  }
  if (request.method === 'GET' && url.pathname === '/') {
    await renderPage(request, response, url);
    return;
  }
  if (request.method === 'POST' && handleTestControl(url, response)) {
    return;
  }
  sendText(response, 404, 'Not Found');
}

async function renderPage(
  request: IncomingMessage,
  response: ServerResponse,
  url: URL,
): Promise<void> {
  if (!acceptsWebUIStream(request.headers.accept)) {
    sendJson(response, { state: STREAMING_STATE });
    return;
  }
  if (
    activeStreams >= options.maxConcurrentStreams ||
    (options.testControls && url.searchParams.get('refuse') === '1')
  ) {
    sendCapacityRefusal(response);
    return;
  }

  const testSession = resolveTestSession(url, response);
  if (options.testControls && !testSession) {
    return;
  }

  activeStreams++;
  response.statusCode = 200;
  response.setHeader('Content-Type', WEBUI_STREAM_MEDIA_TYPE);
  response.setHeader('Cache-Control', 'no-store');
  const abort = new AbortController();
  const abortOnDisconnect = (): void => abort.abort();
  response.once('close', abortOnDisconnect);
  const writer = new WebUIStreamWriter(response);
  try {
    await streamPage(writer, {
      feedDelayMinMs: options.feedDelayMinMs,
      feedDelayMaxMs: options.feedDelayMaxMs,
      testSession,
      signal: abort.signal,
    });
  } catch (error) {
    if (!abort.signal.aborted) {
      const message = error instanceof Error ? error.message : String(error);
      console.error(`streaming API response failed: ${message}`);
    }
    if (!response.destroyed) {
      response.destroy();
    }
  } finally {
    response.off('close', abortOnDisconnect);
    activeStreams--;
  }
}

/**
 * Refuse a stream before any record is written.
 *
 * Shared by the real capacity guard and the `?refuse=1` test control so an
 * end-to-end test observes the identical response a saturated server sends,
 * rather than a parallel code path that could drift from it.
 */
function sendCapacityRefusal(response: ServerResponse): void {
  response.setHeader('Content-Type', WEBUI_STREAM_MEDIA_TYPE);
  sendText(response, 503, 'streaming render capacity is temporarily exhausted');
}

function resolveTestSession(url: URL, response: ServerResponse): TestSession | undefined {
  if (!controls) {
    return undefined;
  }
  const id = url.searchParams.get('test') ?? '';
  const session = controls.session(id);
  if (!session) {
    sendText(response, 400, 'A valid test session is required');
  }
  return session;
}

function handleTestControl(url: URL, response: ServerResponse): boolean {
  if (!controls) {
    return false;
  }
  const segments = url.pathname.split('/');
  if (
    segments.length !== 5 ||
    segments[1] !== 'api' ||
    segments[2] !== '__test'
  ) {
    return false;
  }
  const session = controls.existingSession(segments[3]);
  if (!session) {
    response.statusCode = 404;
    response.end();
    return true;
  }
  switch (segments[4]) {
    case 'feed':
      session.releaseNextFeedGap();
      break;
    case 'weather':
      session.releaseWeather();
      break;
    case 'all':
      session.releaseAll();
      break;
    default:
      response.statusCode = 404;
      response.end();
      return true;
  }
  response.statusCode = 204;
  response.end();
  return true;
}

function sendJson(response: ServerResponse, body: unknown): void {
  response.statusCode = 200;
  response.setHeader('Content-Type', 'application/json; charset=utf-8');
  response.end(JSON.stringify(body));
}

function sendText(response: ServerResponse, status: number, body: string): void {
  response.statusCode = status;
  if (!response.hasHeader('Content-Type')) {
    response.setHeader('Content-Type', 'text/plain; charset=utf-8');
  }
  response.end(body);
}

function failRequest(response: ServerResponse, error: unknown): void {
  const message = error instanceof Error ? error.message : String(error);
  console.error(`streaming API request failed: ${message}`);
  if (response.destroyed) {
    return;
  }
  if (!response.headersSent) {
    sendText(response, 500, 'Internal Server Error');
  } else {
    response.destroy();
  }
}

function parseOptions(args: string[]): ServerOptions {
  const port = numberOption(args, '--port', Number(process.env.PORT) || 3030);
  const feedDelayMinMs = numberOption(args, '--feed-delay-min-ms', 500);
  const feedDelayMaxMs = numberOption(args, '--feed-delay-max-ms', 1_000);
  const maxConcurrentStreams = numberOption(args, '--max-concurrent-streams', 4);
  if (feedDelayMaxMs < feedDelayMinMs) {
    throw new Error('--feed-delay-max-ms must be at least --feed-delay-min-ms');
  }
  if (maxConcurrentStreams < 1) {
    throw new Error('--max-concurrent-streams must be at least 1');
  }
  if (WEATHER_DELAY_MIN_MS <= feedDelayMinMs) {
    throw new Error('the weather delay must outlast the minimum feed delay');
  }
  return {
    port,
    feedDelayMinMs,
    feedDelayMaxMs,
    maxConcurrentStreams,
    testControls: args.includes('--test-controls'),
  };
}

function numberOption(args: string[], name: string, fallback: number): number {
  const index = args.indexOf(name);
  if (index === -1) {
    return fallback;
  }
  const value = Number(args[index + 1]);
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new Error(`${name} requires a non-negative integer`);
  }
  return value;
}
