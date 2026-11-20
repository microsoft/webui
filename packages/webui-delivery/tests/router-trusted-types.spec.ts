// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { randomBytes } from 'node:crypto';
import { expect, test } from '@playwright/test';
import type { PartialResponse } from '../src/cache.js';

for (const transport of ['JSON', 'NDJSON'] as const) {
  test(`cold ${transport} navigation runs compiled conditions under Trusted Types and nonce CSP`, async ({ page }) => {
    const nonce = randomBytes(16).toString('base64');
    const policy = `default-src 'self'; script-src 'nonce-${nonce}'; style-src 'self' 'nonce-${nonce}'; require-trusted-types-for 'script'; trusted-types webui`;
    const errors: string[] = [];
    page.on('pageerror', error => errors.push(error.message));
    page.on('console', message => {
      const text = message.text();
      // The SSR fixture emits route-wrapper style attributes, which this CSP blocks.
      if (message.type() === 'error' && !text.startsWith('Applying inline style')) errors.push(text);
      if (message.type() === 'warning' && text.startsWith('[Router]')) errors.push(text);
    });

    await page.route('**/alpha?*', async route => {
      // The CLI fixture serves compiled SSR but does not set a request nonce.
      const response = await route.fetch();
      const body = (await response.text())
        .replace('</head>', `<meta name="webui-nonce" content="${nonce}"></head>`)
        .replace(
          '<script type="application/json" id="webui-data">{',
          `<script type="application/json" id="webui-data">{"nonce":"${nonce}",`,
        )
        .replace(/<(script|style)(?=[\s>])/g, `<$1 nonce="${nonce}"`);
      await route.fulfill({
        response,
        headers: { ...response.headers(), 'content-security-policy': policy },
        body,
      });
    });

    if (transport === 'NDJSON') {
      await page.route('**/keepalive', async route => {
        const headers = route.request().headers();
        if (!headers.accept?.includes('application/x-ndjson')) {
          await route.continue();
          return;
        }
        // Frame the CLI's actual compiler payload as NDJSON; keep the templates and closures intact.
        const response = await route.fetch({ headers: { ...headers, accept: 'application/json' } });
        const { state, ...metadata } = await response.json() as PartialResponse;
        await route.fulfill({
          response,
          contentType: 'application/x-ndjson',
          body: `${JSON.stringify(metadata)}\n${JSON.stringify({ states: [null, state ?? null] })}\n`,
        });
      });
    }

    const documentResponse = await page.goto(transport === 'JSON' ? '/alpha?cache=zero' : '/alpha?cache=zero-stream');
    expect(await documentResponse?.headerValue('content-security-policy')).toBe(policy);
    await page.waitForFunction(() => {
      const router = (window as Window & { __testRouter?: { activeComponent: string } }).__testRouter;
      return router?.activeComponent === 'page-alpha';
    });
    expect(await page.evaluate(() => {
      document.body.dataset.instance = 'same-document';
      const runtime = window.__webui as
        | (WebUIRuntimeGlobal & { templates?: Record<string, unknown> })
        | undefined;
      return {
        nonce: runtime?.nonce,
        hasDestination: Object.hasOwn(runtime?.templates ?? {}, 'page-keepalive'),
      };
    })).toEqual({ nonce, hasDestination: false });

    const partial = page.waitForResponse(response =>
      new URL(response.url()).pathname === '/keepalive' &&
      response.headers()['content-type']?.includes(transport === 'JSON' ? 'application/json' : 'application/x-ndjson'),
    );
    await page.getByRole('link', { name: 'Keep-Alive' }).click();
    const response = await partial;
    expect(response.ok()).toBe(true);
    const metadata = (transport === 'JSON'
      ? await response.json()
      : JSON.parse((await response.text()).split('\n', 1)[0])) as PartialResponse;
    expect(metadata.templateFunctions?.['page-keepalive']).toBeTruthy();

    try {
      await expect(page.getByRole('heading', { name: 'Keep-Alive Page' })).toBeVisible();
    } finally {
      expect.soft(errors).toEqual([]);
    }
    await expect(page.getByTestId('incremented')).toHaveCount(0);
    await page.getByRole('button', { name: 'Increment' }).click();
    await expect(page.getByTestId('incremented')).toHaveText('Counter updated');
    await expect(page.locator('body')).toHaveAttribute('data-instance', 'same-document');
    expect(errors).toEqual([]);
  });
}
