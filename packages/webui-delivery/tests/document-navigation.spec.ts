// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test, type Page } from '@playwright/test';
import { build, Protocol } from '@microsoft/webui';
import { createServer, type Server } from 'node:http';
import { readFileSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import type { AddressInfo } from 'node:net';
import type { RouterConfig, WebUIRouter } from '@microsoft/webui-delivery/router.js';

interface FixtureWindow extends Window {
  __testRouter: WebUIRouter;
  __testRouterConfig: RouterConfig;
  __startRouter(): void;
  __ownership: { inspected: number; errors: number; navigated: number };
}

let server: Server;
let origin: string;
const requests: Array<{ path: string; partial: boolean }> = [];

test.beforeAll(async () => {
  const compiled = build({
    appDir: fileURLToPath(new URL('fixtures/router-app/src', import.meta.url)),
    entry: 'document.html', plugin: 'webui', css: 'style',
  });
  const protocol = new Protocol(compiled.protocol, { plugin: 'webui' });
  const dist = new URL('fixtures/router-app/dist/', import.meta.url);
  const scripts = new Map(readdirSync(dist).filter(name => name.endsWith('.js'))
    .map(name => [name, readFileSync(new URL(name, dist))]));
  server = createServer((request, response) => {
    const url = new URL(request.url ?? '/', 'http://localhost');
    const base = url.pathname.startsWith('/app/') ? '/app' : '';
    const path = url.pathname.slice(base.length);
    const script = scripts.get(path.slice(1));
    if (script) {
      response.writeHead(200, { 'content-type': 'text/javascript' }).end(script);
      return;
    }
    if (path === '/favicon.ico') { response.writeHead(204).end(); return; }
    const partial = request.headers.accept?.includes('json') ?? false;
    requests.push({ path: url.pathname + url.search, partial });
    const state = { basePath: base + '/' };
    if (partial && path !== '/settings' && path !== '/settings/profile') {
      response.writeHead(200, { 'content-type': 'application/json' }).end(
        protocol.renderPartial(state, 'document.html', path,
          String(request.headers['x-webui-inventory'] ?? '')),
      );
    } else {
      response.writeHead(200, { 'content-type': 'text/html' }).end(
        protocol.render(state, { entry: 'document.html', requestPath: path }),
      );
    }
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  origin = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
});

test.afterAll(async () => {
  server.closeAllConnections();
  await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
});

async function open(page: Page, options: { fallback?: boolean; base?: string; prepare?: boolean } = {}) {
  await page.addInitScript(({ fallback }) => {
    if (fallback) Object.defineProperty(window, 'navigation', { value: undefined, configurable: true });
    const globals = window as unknown as FixtureWindow;
    const state = globals.__ownership = { inspected: 0, errors: 0, navigated: 0 };
    const config: RouterConfig = {
      shouldIntercept(url: URL) {
        const path = url.pathname.startsWith('/app/') ? url.pathname.slice(4) : url.pathname;
        return path === '/settings/performance' || path === '/alpha' || path === '/beta';
      },
      responsePolicy: {
        inspect(response: Response) {
          state.inspected++;
          if (response.headers.get('content-type')?.includes('text/html')) {
            throw new Error('Classified authentication HTML');
          }
        },
        onError() { state.errors++; },
      },
    };
    globals.__testRouterConfig = config;
    window.addEventListener('webui:route:navigated', () => { state.navigated++; });
  }, { fallback: options.fallback });
  await page.goto(`${origin}${options.base ?? ''}/settings/performance${options.prepare ? '?prepare' : ''}`);
  if (options.prepare) {
    await page.waitForFunction(() => typeof (window as unknown as FixtureWindow).__startRouter === 'function');
  } else {
    await page.waitForFunction(() => (window as unknown as FixtureWindow).__ownership.navigated === 1);
  }
  requests.length = 0;
  await page.locator('body').evaluate(body => { body.dataset.instance = 'edited-document'; });
}

async function protectEdits(page: Page): Promise<void> {
  const content = page.locator('.page-beta .content');
  await content.evaluate(element => { element.setAttribute('contenteditable', 'true'); });
  await content.fill('Unsaved native edit');
  await page.locator('h2').click();
  await page.evaluate(() => {
    window.addEventListener('beforeunload', event => {
      event.preventDefault();
      event.returnValue = '';
    });
  });
}

for (const reload of ['location', 'navigation'] as const) {
  test(`${reload}.reload performs a real document request, not classified partial revalidation`, async ({ page }) => {
    await open(page);
    const requested = page.waitForRequest(request =>
      new URL(request.url()).pathname === '/settings/performance',
    );
    await page.evaluate(reload => {
      if (reload === 'location') location.reload();
      else window.navigation.reload();
    }, reload);
    expect((await requested).isNavigationRequest()).toBe(true);
    await expect.poll(() => requests).toContainEqual({ path: '/settings/performance', partial: false });
    expect(requests.some(request => request.partial)).toBe(false);
    await expect(page.locator('body')).not.toHaveAttribute('data-instance', 'edited-document');
  });
}

for (const fallback of [false, true]) {
  test(`canceled reload preserves the installed router (${fallback ? 'fallback' : 'native'})`, async ({ page }) => {
    await open(page, { fallback });
    await protectEdits(page);
    const dialog = page.waitForEvent('dialog');
    await page.evaluate(() => { setTimeout(() => location.reload(), 0); });
    await (await dialog).dismiss();
    await expect(page.locator('body')).toHaveAttribute('data-instance', 'edited-document');
    await expect(page.locator('.page-beta .content')).toHaveText('Unsaved native edit');
    expect(new URL(page.url()).pathname).toBe('/settings/performance');
    await page.evaluate(() => (window as unknown as FixtureWindow).__testRouter.navigate('/alpha?after=cancel'));
    await expect(page.locator('h2')).toHaveText('Alpha Page');
    expect(requests).toEqual([{ path: '/alpha?after=cancel', partial: true }]);
  });

  test(`exact document ownership preserves a native leaf and base/query (${fallback ? 'fallback' : 'native'})`, async ({ page }) => {
    await open(page, { fallback, base: '/app' });
    await page.evaluate(() => (window as unknown as FixtureWindow).__testRouter.navigate('/settings/performance?refresh=1'));
    await expect.poll(() => requests).toContainEqual({
      path: '/app/settings/performance?refresh=1', partial: true,
    });
    await expect(page.locator('body')).toHaveAttribute('data-instance', 'edited-document');
    await expect(page.locator('h2')).toHaveText('Beta Page');
    if (!fallback) {
      requests.length = 0;
      await page.evaluate(async () => { await window.navigation.navigate(location.href).finished; });
      expect(requests).toEqual([{ path: '/app/settings/performance?refresh=1', partial: true }]);
      await expect(page.locator('body')).toHaveAttribute('data-instance', 'edited-document');
    }
    const loaded = page.waitForEvent('load');
    await page.evaluate(() => (window as unknown as FixtureWindow).__testRouter.navigate('/settings?from=leaf'));
    await loaded;
    expect(requests).toContainEqual({ path: '/app/settings?from=leaf', partial: false });
    expect(requests.some(request => request.path.includes('/settings?') && request.partial)).toBe(false);
    await expect(page.locator('body')).not.toHaveAttribute('data-instance', 'edited-document');
  });

  test(`canceled document crossing leaves native routing usable (${fallback ? 'fallback' : 'native'})`, async ({ page }) => {
    await open(page, { fallback });
    await protectEdits(page);
    const dialog = page.waitForEvent('dialog');
    await page.evaluate(() => {
      setTimeout(() => (window as unknown as FixtureWindow).__testRouter.navigate('/settings/profile'), 0);
    });
    await (await dialog).dismiss();
    await expect(page.locator('body')).toHaveAttribute('data-instance', 'edited-document');
    await expect(page.locator('.page-beta .content')).toHaveText('Unsaved native edit');
    expect(new URL(page.url()).pathname).toBe('/settings/performance');
    await page.evaluate(() => (window as unknown as FixtureWindow).__testRouter.navigate('/alpha'));
    await expect(page.locator('h2')).toHaveText('Alpha Page');
    expect(requests).toEqual([{ path: '/alpha', partial: true }]);
    expect(await page.evaluate(() => (window as unknown as FixtureWindow).__ownership.errors)).toBe(0);
  });

  test(`ordinary external links stay document navigations (${fallback ? 'fallback' : 'native'})`, async ({ page, baseURL }) => {
    await open(page, { fallback });
    if (!baseURL) throw new Error('The public fixture server must be available');
    const link = page.getByRole('link', { name: 'Alpha', exact: true });
    await link.evaluate((anchor, href) => anchor.setAttribute('href', href), `${baseURL}/alpha`);
    const external = page.waitForRequest(request => request.url() === `${baseURL}/alpha`);
    await link.click();
    expect((await external).isNavigationRequest()).toBe(true);
    await expect(page.locator('h2')).toHaveText('Alpha Page');
    expect(requests).toEqual([]);
  });
}

for (const prepare of [false, true]) {
  test(`document-owned intent is not fetched, classified or cached (${prepare ? 'prepared' : 'active'})`, async ({ page }) => {
    await open(page, { prepare });
    const link = page.getByRole('link', { name: 'Alpha', exact: true });
    await link.evaluate(anchor => anchor.setAttribute('href', '/settings'));
    await link.hover();
    // Let the delegated pointer handler and any wrongly started request reach the server.
    await page.waitForTimeout(100);
    expect(requests).toEqual([]);
    expect(await page.evaluate(() => (window as unknown as FixtureWindow).__ownership.inspected)).toBe(0);
    if (prepare) await page.evaluate(() => (window as unknown as FixtureWindow).__startRouter());
    await link.evaluate(anchor => anchor.setAttribute('href', '/alpha'));
    await link.click();
    await expect(page.locator('h2')).toHaveText('Alpha Page');
    expect(requests).toContainEqual({ path: '/alpha', partial: true });
    expect(await page.evaluate(() => (window as unknown as FixtureWindow).__ownership.errors)).toBe(0);
  });
}
