// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { createServer, request, type Server, type ServerResponse } from 'node:http';
import { writeFile } from 'node:fs/promises';
import { test, expect } from '@playwright/test';
import type { PartialResponse } from '../src/cache.js';

type StreamingTestWindow = typeof window & {
  __webui?: {
    templates?: Record<string, unknown>;
    componentStyles?: PartialResponse['componentStyles'];
  };
  __testRouter: {
    activeComponent: string;
    startupNavigation: Promise<void> | null;
    deferredReader: Promise<void> | null;
    partialControllers: Set<AbortController>;
    navCache: {
      has(path: string): boolean;
      getEntry(path: string): { complete: boolean; data: PartialResponse } | undefined;
      tagIndex: Map<string, Set<string>>;
    };
  };
  __streamStateCalls?: string[][];
};

async function listenLoopback(server: Server): Promise<URL> {
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('fixture must bind TCP');
  return new URL(`http://127.0.0.1:${address.port}`);
}

async function fetchFixtureResponse(
  target: string,
  backend: URL,
  headers: Headers,
): Promise<Response> {
  const query = target.indexOf('?');
  const pathname = query < 0 ? target : target.slice(0, query);
  if (
    !pathname.startsWith('/') || pathname.startsWith('//') ||
    pathname.includes('\\') || pathname.includes('#')
  ) {
    return new Response('Expected an origin-form request target', {
      status: 400,
      headers: {
        'Content-Type': 'text/plain; charset=utf-8',
        'X-Content-Type-Options': 'nosniff',
      },
    });
  }
  const upstream = new URL(backend.href);
  upstream.pathname = pathname;
  upstream.search = query < 0 ? '' : target.slice(query);
  upstream.hash = '';
  return fetch(upstream, { headers, redirect: 'manual' });
}

async function requestTarget(
  origin: URL,
  target: string,
): Promise<{
  status: number;
  body: string;
  contentType: string | undefined;
  contentTypeOptions: string | string[] | undefined;
}> {
  return new Promise((resolve, reject) => {
    const outgoing = request({
      hostname: origin.hostname,
      port: origin.port,
      path: target,
    }, response => {
      const status = response.statusCode;
      if (status === undefined) {
        response.resume();
        reject(new Error('fixture response must include an HTTP status'));
        return;
      }
      let body = '';
      response.setEncoding('utf8');
      response.on('data', chunk => { body += chunk; });
      response.on('error', reject);
      response.on('end', () => resolve({
        status, body,
        contentType: response.headers['content-type'],
        contentTypeOptions: response.headers['x-content-type-options'],
      }));
    });
    outgoing.on('error', reject);
    outgoing.setTimeout(10_000, () => outgoing.destroy(new Error('fixture request timed out')));
    outgoing.end();
  });
}

test('streaming fixture proxy rejects upstream origin escapes', async ({}, testInfo) => {
  const sentinelTargets: string[] = [];
  const intendedTargets: string[] = [];
  const failures: Error[] = [];
  const sentinel = createServer((request, response) => {
    sentinelTargets.push(request.url!);
    response.setHeader('Content-Type', 'text/plain; charset=utf-8');
    response.setHeader('X-Content-Type-Options', 'nosniff');
    response.end('unexpected sentinel upstream');
  });
  const servers = [sentinel];
  try {
    const sentinelOrigin = await listenLoopback(sentinel);
    const intended = createServer((request, response) => {
      intendedTargets.push(request.url!);
      response.setHeader('Content-Type', 'text/plain; charset=utf-8');
      response.setHeader('X-Content-Type-Options', 'nosniff');
      if (request.url === '/redirect') {
        response.writeHead(302, { location: `${sentinelOrigin.origin}/redirected` });
      }
      response.end(request.url);
    });
    servers.push(intended);
    const intendedOrigin = await listenLoopback(intended);
    const proxy = createServer((request, response) => {
      void (async () => {
        const upstream = await fetchFixtureResponse(
          request.url!, intendedOrigin, new Headers({ accept: 'text/plain' }),
        );
        response.writeHead(upstream.status, {
          'Content-Type': 'text/plain; charset=utf-8',
          'X-Content-Type-Options': 'nosniff',
        });
        response.end(await upstream.text());
      })().catch(error => {
        failures.push(error);
        response.destroy(error);
      });
    });
    servers.push(proxy);
    const proxyOrigin = await listenLoopback(proxy);
    const normalTarget = '/normal%20path?value=a%2Fb&value=2&value=%25';
    const normal = await requestTarget(proxyOrigin, normalTarget);
    const relative = await requestTarget(proxyOrigin, `//${sentinelOrigin.host}/relative`);
    const absolute = await requestTarget(proxyOrigin, `${sentinelOrigin.origin}/absolute`);
    const redirect = await requestTarget(proxyOrigin, '/redirect');
    const ledgerPath = testInfo.outputPath('proxy-origin-regression.json');
    await writeFile(ledgerPath, JSON.stringify({
      intendedOrigin, sentinelOrigin, proxyOrigin, normal, relative, absolute, redirect,
      intendedTargets, sentinelTargets,
    }, null, 2));
    await testInfo.attach('proxy-origin-regression', {
      path: ledgerPath, contentType: 'application/json',
    });
    expect(normal).toEqual({
      status: 200, body: normalTarget,
      contentType: 'text/plain; charset=utf-8', contentTypeOptions: 'nosniff',
    });
    for (const result of [normal, relative, absolute, redirect]) {
      expect(result.contentType).toBe('text/plain; charset=utf-8');
      expect(result.contentTypeOptions).toBe('nosniff');
    }
    expect.soft(relative.status).toBe(400);
    expect.soft(absolute.status).toBe(400);
    expect.soft(redirect.status).toBe(302);
    expect.soft(sentinelTargets).toEqual([]);
    expect(intendedTargets).toEqual([normalTarget, '/redirect']);
    expect(failures).toEqual([]);
  } finally {
    for (const server of servers) {
      server.closeAllConnections();
      if (!server.listening) continue;
      await new Promise<void>((resolve, reject) =>
        server.close(error => error ? reject(error) : resolve()),
      );
    }
  }
});

for (const terminal of ['eof', 'abort', 'error'] as const) {
  test(`ordinary zero-freshness NDJSON releases ownership after ${terminal}`, async ({
    page,
    baseURL,
  }, testInfo) => {
    if (!baseURL) throw new Error('router fixture requires a configured baseURL');
    const backend = new URL(baseURL);
    const tail = Promise.withResolvers<ServerResponse>();
    const closed = Promise.withResolvers<void>();
    const requests: Array<{ path: string; accept: string; inventory: string }> = [];
    const wireRecords: string[] = [];
    const consoleMessages: Array<{ type: string; text: string }> = [];
    const pageErrors: string[] = [];
    let revision = 0;
    const failures: Error[] = [];
    const server = createServer((request, response) => {
      void (async () => {
        const path = request.url!;
        const headers = new Headers();
        for (const [name, value] of Object.entries(request.headers)) {
          if (value !== undefined && name !== 'host' && name !== 'connection') {
            headers.set(name, Array.isArray(value) ? value.join(', ') : value);
          }
        }
        const accept = headers.get('accept') ?? '';
        requests.push({ path, accept, inventory: headers.get('x-webui-inventory') ?? '' });
        const upstream = await fetchFixtureResponse(path, backend, headers);
        if (path === '/items/1' && accept.includes('application/json')) {
          const partial = await upstream.json() as PartialResponse;
          revision++;
          delete partial.state;
          partial.cacheTags = ['stream-owner'];
          response.writeHead(200, {
            'content-type': 'application/x-ndjson',
            'cache-control': 'no-store',
          });
          const first = `${JSON.stringify(partial)}\n`;
          wireRecords.push(first);
          response.write(first);
          response.on('close', () => closed.resolve());
          if (revision === 1) {
            tail.resolve(response);
          } else {
            const last = `${JSON.stringify({
              states: [null, { itemId: `1 revision ${revision}` }],
            })}\n`;
            wireRecords.push(last);
            response.end(last);
          }
          return;
        }
        response.writeHead(upstream.status, {
          'content-type': upstream.headers.get('content-type') ?? 'application/octet-stream',
          'cache-control': 'no-store',
        });
        response.end(Buffer.from(await upstream.arrayBuffer()));
      })().catch(error => {
        failures.push(error);
        response.destroy(error);
      });
    });
    const origin = (await listenLoopback(server)).origin;
    page.on('console', message => consoleMessages.push({
      type: message.type(), text: message.text(),
    }));
    page.on('pageerror', error => pageErrors.push(error.message));

    try {
      await page.goto(`${origin}/beta?cache=zero`);
      await page.waitForFunction(() => {
        const router = (window as StreamingTestWindow).__testRouter;
        return router?.activeComponent === 'page-beta' && router.startupNavigation === null;
      });
      await page.keyboard.press('Tab');
      await expect(page.locator('nav a[href="/"]')).toBeFocused();
      for (let i = 0; i < 6; i++) await page.keyboard.press('Tab');
      await expect(page.locator('nav a[href="/items/1"]')).toBeFocused();
      await page.keyboard.press('Enter');
      const response = await tail.promise;
      await page.waitForFunction(() =>
        (window as StreamingTestWindow).__testRouter.activeComponent === 'page-detail',
      );
      await page.waitForFunction(() =>
        (window as StreamingTestWindow).__testRouter.deferredReader !== null,
      );
      expect(await page.evaluate(() => {
        const cache = (window as StreamingTestWindow).__testRouter.navCache;
        return { held: cache.has('/items/1'), complete: cache.getEntry('/items/1')?.complete };
      })).toEqual({ held: true, complete: false });
      await page.evaluate(() => {
        const runtime = window as StreamingTestWindow;
        runtime.__streamStateCalls = [];
        const component = document.querySelector('route-shell')?.shadowRoot
          ?.querySelector<HTMLElement & { setState(state: Record<string, unknown>): void }>(
            'page-detail',
          );
        if (!component) throw new Error('detail route must mount before its tail');
        const setState = component.setState.bind(component);
        component.setState = state => {
          runtime.__streamStateCalls!.push(Object.keys(state));
          setState(state);
        };
      });

      if (terminal === 'eof') {
        const last = `${JSON.stringify({
          states: [null, { itemId: '1 revision 1', streamPayload: 'G2-owner:'.repeat(8_192) }],
        })}\n`;
        wireRecords.push(last);
        response.end(last);
        await expect(page.locator('h2')).toHaveText('Item 1 revision 1');
      } else if (terminal === 'error') {
        response.destroy();
      }
      if (terminal !== 'abort') {
        await page.waitForFunction(() =>
          (window as StreamingTestWindow).__testRouter.deferredReader === null,
        );
      }
      await page.waitForFunction(() => document.getAnimations().length === 0);
      const betaLink = page.locator('nav a[href="/beta"]');
      for (let i = 0; i < 16; i++) {
        await page.keyboard.press('Shift+Tab');
        if (await betaLink.evaluate(link => link.matches(':focus'))) break;
      }
      await expect(betaLink).toBeFocused();
      await page.keyboard.press('Enter');
      await expect(page.locator('h2')).toHaveText('Beta Page');
      await closed.promise;
      await page.waitForFunction(() =>
        (window as StreamingTestWindow).__testRouter.deferredReader === null,
      );
      await page.waitForFunction(() => document.getAnimations().length === 0);
      const ownership = await page.evaluate(() => {
        const runtime = window as StreamingTestWindow;
        const router = runtime.__testRouter;
        const entry = router.navCache.getEntry('/items/1');
        const shell = document.querySelector<HTMLElement & { $ready?: boolean }>('route-shell');
        const beta = shell?.shadowRoot?.querySelector<HTMLElement & { $ready?: boolean }>(
          'page-beta',
        );
        return {
          retained: router.navCache.has('/items/1'),
          complete: entry?.complete,
          stateKeys: Object.keys(entry?.data.chain?.at(-1)?.state ?? {}),
          tags: [...router.navCache.tagIndex].map(([tag, paths]) => [tag, [...paths]]),
          controllers: router.partialControllers.size,
          stateCalls: runtime.__streamStateCalls,
          templates: Object.keys(runtime.__webui?.templates ?? {}).sort(),
          templateMetadata: runtime.__webui?.templates,
          componentStyles: runtime.__webui?.componentStyles,
          inventory: window.__webui?.inventory,
          readiness: { shell: shell?.$ready, beta: beta?.$ready },
          markup: shell?.shadowRoot?.innerHTML,
          styles: [...document.querySelectorAll('link[rel="stylesheet"]')]
            .map(link => link.getAttribute('href')),
        };
      });
      const ledgerPath = testInfo.outputPath('settled-ownership.json');
      await writeFile(ledgerPath, JSON.stringify({
        terminal, ownership, requests, wireRecords, consoleMessages, pageErrors,
      }, null, 2));
      await testInfo.attach('settled-ownership', {
        path: ledgerPath, contentType: 'application/json',
      });
      await page.screenshot({ path: testInfo.outputPath('settled-desktop.png') });
      expect(failures).toEqual([]);
      expect(pageErrors).toEqual([]);
      expect(ownership.readiness).toEqual({ shell: true, beta: true });
      expect(ownership.controllers).toBe(0);
      expect(ownership.stateCalls).toEqual(terminal === 'eof'
        ? [['itemId', 'streamPayload']] : []);
      expect.soft(ownership.retained).toBe(false);
      expect.soft(ownership.tags).toEqual([]);

      await page.goBack();
      await expect(page.locator('h2')).toHaveText('Item 1 revision 2');
      await page.goForward();
      await expect(page.locator('h2')).toHaveText('Beta Page');
      expect(revision).toBe(2);
      await page.setViewportSize({ width: 390, height: 844 });
      await page.waitForFunction(() => document.getAnimations().length === 0);
      await page.screenshot({ path: testInfo.outputPath('settled-narrow.png') });
      await writeFile(ledgerPath, JSON.stringify({
        terminal, ownership, requests, wireRecords, consoleMessages, pageErrors,
      }, null, 2));
      expect(failures).toEqual([]);
      expect(pageErrors).toEqual([]);
      const warnings = consoleMessages.filter(message =>
        message.type === 'warning' || message.type === 'error',
      );
      if (terminal === 'error') {
        expect(warnings.some(message => message.text.includes('Deferred state reader failed')))
          .toBe(true);
        expect(warnings.every(message =>
          message.text.includes('Deferred state reader failed') ||
          message.text.includes('ERR_INCOMPLETE_CHUNKED_ENCODING'),
        )).toBe(true);
      } else {
        expect(warnings).toEqual([]);
      }
    } finally {
      server.closeAllConnections();
      await new Promise<void>((resolve, reject) =>
        server.close(error => error ? reject(error) : resolve()),
      );
    }
  });
}
