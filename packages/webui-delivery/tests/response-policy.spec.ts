// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';
import { build, Protocol } from '@microsoft/webui';
import { createServer } from 'node:http';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import type { AddressInfo } from 'node:net';
import type { PartialResponse } from '@microsoft/webui-delivery/router.js';

test.beforeEach(async ({ page }) => {
  await page.route('**/index.js', async route => {
    const response = await route.fetch({
      url: new URL('/lean-entry.js', route.request().url()).href,
    });
    await route.fulfill({ response });
  });
  await page.addInitScript(() => {
    const state = {
      errors: [] as string[],
      responses: [] as Array<{ status: number; type: string }>,
      navigated: 0,
    };
    Object.assign(window, {
      __recovery: state,
      __testRouterConfig: {
        responsePolicy: {
          redirect: 'manual',
          inspect(response: Response) {
            state.responses.push({ status: response.status, type: response.type });
            if (response.type === 'opaqueredirect' || !response.ok ||
                response.headers.get('content-type')?.includes('text/html')) {
              throw new Error('Manual recovery required');
            }
          },
          onError(error: unknown) {
            state.errors.push(error instanceof Error ? error.message : String(error));
          },
        },
      },
    });
    window.addEventListener('webui:route:navigated', () => { state.navigated++; });
  });
  await page.goto('/alpha');
  await page.waitForFunction(() =>
    (window as unknown as { __recovery: { navigated: number } }).__recovery.navigated === 1,
  );
  await page.locator('.page-alpha .content').evaluate(element => {
    element.setAttribute('contenteditable', 'true');
  });
  await page.locator('.page-alpha .content').fill('Unsaved edit');
});

for (const kind of ['403', 'html', 'redirect', 'network', 'same-url']) {
  test(`keeps edited document and rejects navigation on ${kind}`, async ({ page }) => {
    let documents = 0;
    page.on('request', request => { if (request.isNavigationRequest()) documents++; });
    const destination = kind === 'same-url' ? '/alpha' : '/beta';
    await page.route(`**${destination}`, async route => {
      if (kind === 'network') await route.abort('failed');
      else if (kind === 'redirect') {
        await route.fulfill({ status: 302, headers: { location: '/sign-in' } });
      } else {
        await route.fulfill({
          status: kind === '403' || kind === 'same-url' ? 403 : 200,
          contentType: 'text/html',
          body: '<!doctype html><title>Sign in</title>',
        });
      }
    });
    const result = await page.evaluate(async destination => {
      try {
        await window.navigation.navigate(destination).finished;
        return 'fulfilled';
      } catch {
        return 'rejected';
      }
    }, destination);
    expect(result).toBe('rejected');
    await expect(page.locator('.page-alpha .content')).toHaveText('Unsaved edit');
    expect(documents).toBe(0);
    const state = await page.evaluate(() =>
      (window as unknown as { __recovery: {
        errors: string[]; navigated: number; responses: Array<{ status: number; type: string }>;
      } }).__recovery,
    );
    expect(state.errors).toHaveLength(1);
    expect(state.navigated).toBe(1);
    if (kind === 'redirect') {
      expect(state.responses).toEqual([{ status: 0, type: 'opaqueredirect' }]);
    }
  });
}

test('ordinary compiled partial navigation preserves its parent', async ({ page }) => {
  await page.locator('route-shell').evaluate(element => {
    element.setAttribute('data-instance', 'retained');
  });
  await page.evaluate(async () => { await window.navigation.navigate('/beta').finished; });
  await expect(page.locator('h2')).toHaveText('Beta Page');
  await expect(page.locator('route-shell')).toHaveAttribute('data-instance', 'retained');
});

test('fallback navigation reports recovery without replacing the document', async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(window, 'navigation', { value: undefined, configurable: true });
  });
  await page.reload();
  await page.waitForFunction(() =>
    (window as unknown as { __recovery: { navigated: number } }).__recovery.navigated === 1,
  );
  await page.route('**/beta', route => route.fulfill({ status: 403, body: 'Denied' }));
  await page.locator('nav a[href="/beta"]').click();
  await expect.poll(() => page.evaluate(() =>
    (window as unknown as { __recovery: { errors: string[] } }).__recovery.errors.length,
  )).toBe(1);
  await expect(page.locator('h2')).toHaveText('Alpha Page');
});

for (const outcome of ['complete', 'truncated', 'superseded'] as const) {
  test(`public Node partial framing delivers resources before state: ${outcome}`, async ({ page }) => {
    const compiled = build({
      appDir: fileURLToPath(new URL('fixtures/router-app/src', import.meta.url)),
      plugin: 'webui', css: 'style',
    });
    const protocol = new Protocol(compiled.protocol, { plugin: 'webui' });
    const script = readFileSync(new URL('fixtures/router-app/dist/lean-entry.js', import.meta.url));
    const tail = Promise.withResolvers<void>();
    const closed = Promise.withResolvers<void>();
    let response: PartialResponse | undefined;
    let revision = 0;
    const server = createServer((request, res) => {
      const path = new URL(request.url ?? '/', 'http://localhost').pathname;
      if (path.endsWith('.js')) {
        res.writeHead(200, { 'content-type': 'text/javascript' }).end(script);
        return;
      }
      if (!request.headers.accept?.includes('json')) {
        res.writeHead(200, { 'content-type': 'text/html' }).end(
          protocol.render({}, { requestPath: path }),
        );
        return;
      }
      const inventory = String(request.headers['x-webui-inventory'] ?? '');
      const partialJson = protocol.renderPartial({
        action: 'reply', to: 'reader@example.test', subject: `Late server state ${++revision}`,
        isAlpha: false,
      }, 'index.html', path, inventory);
      if (path !== '/compose') {
        res.writeHead(200, { 'content-type': 'application/json' }).end(partialJson);
        return;
      }
      // Framing framework output does not recreate routes, resources, or projection.
      // The state is already buffered: only its delivery is delayed by this host.
      response = JSON.parse(partialJson) as PartialResponse;
      const { state, ...resources } = response;
      res.writeHead(200, { 'content-type': 'application/x-ndjson' });
      res.write(JSON.stringify(resources) + '\n');
      res.on('close', () => closed.resolve());
      void tail.promise.then(() => {
        res.end(outcome === 'truncated' ? '' : JSON.stringify({
          states: resources.chain?.map((_entry, level, chain) =>
            level === chain.length - 1 ? state ?? null : null),
        }) + '\n');
      });
    });
    await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address() as AddressInfo;
    try {
      await page.goto(`http://127.0.0.1:${address.port}/alpha?ndjson`);
      await page.waitForFunction(() =>
        (window as unknown as { __recovery: { navigated: number } }).__recovery.navigated === 1,
      );
      await page.locator('route-shell').evaluate(element => {
        element.setAttribute('data-instance', 'retained');
        (element as HTMLElement & { isAlpha: boolean }).isAlpha = true;
      });
      await expect(page.locator('nav a[href="/alpha"]')).toHaveAttribute('active', '');
      await page.evaluate(() => {
        const state = window as unknown as { __finished?: string };
        state.__finished = 'pending';
        const finished = window.navigation.navigate('/compose').finished;
        if (!finished) throw new Error('Navigation API completion is required by this fixture');
        void finished.then(
          () => { state.__finished = 'fulfilled'; },
          () => { state.__finished = 'rejected'; },
        );
      });
      await expect(page.locator('h2')).toHaveText('Compose');
      await expect(page.locator('route-shell')).toHaveAttribute('data-instance', 'retained');
      expect(response?.chain?.map(entry => entry.component)).toEqual(['route-shell', 'page-compose']);
      expect(response?.chain?.every(entry => !Object.hasOwn(entry, 'state'))).toBe(true);
      expect(response?.templates).toHaveProperty('page-compose');
      expect(response?.componentStyles).toBeDefined();
      expect(await page.evaluate(() =>
        (window as unknown as { __finished: string }).__finished,
      )).toBe('pending');
      if (outcome === 'superseded') {
        await page.evaluate(async () => { await window.navigation.navigate('/beta').finished; });
        await closed.promise;
      }
      tail.resolve();
      await expect.poll(() => page.evaluate(() =>
        (window as unknown as { __finished: string }).__finished,
      )).toBe(outcome === 'complete' ? 'fulfilled' : 'rejected');
      if (outcome === 'complete') {
        await expect(page.locator('.subject')).toHaveText('Subject: Late server state 1');
        await expect(page.locator('nav a[href="/alpha"]')).toHaveAttribute('active', '');
        await page.evaluate(async () => {
          await window.navigation.navigate(location.href).finished;
        });
        await expect(page.locator('.subject')).toHaveText('Subject: Late server state 2');
        await expect(page.locator('route-shell')).toHaveAttribute('data-instance', 'retained');
        await expect(page.locator('nav a[href="/alpha"]')).toHaveAttribute('active', '');
      }
      if (outcome === 'superseded') await expect(page.locator('h2')).toHaveText('Beta Page');
      const errors = await page.evaluate(() =>
        (window as unknown as { __recovery: { errors: string[] } }).__recovery.errors,
      );
      expect(errors).toHaveLength(outcome === 'truncated' ? 1 : 0);
    } finally {
      tail.resolve();
      server.closeAllConnections();
      await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
    }
  });
}
