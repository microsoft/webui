// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';
import { build } from 'esbuild';
import { fileURLToPath } from 'node:url';
import type { WebUIRouter } from '../src/router.js';

declare global {
  interface Window {
    routerTrustTest: {
      ensureLoaded: WebUIRouter['ensureLoaded'];
      fetchPartial(path: string): Promise<unknown>;
    };
  }
}

let client: string;

test.beforeAll(async () => {
  const result = await build({
    stdin: {
      contents: `
        import { configureTrustedTypes } from '@microsoft/webui-framework/trusted-types.js';
        configureTrustedTypes('router-compiled');
        const { WebUIRouter } = await import('./src/router.ts');
        window.__webui = { inventory: 'before', nonce: 'document-nonce' };
        window.routerTrustTest = new WebUIRouter();
      `,
      resolveDir: fileURLToPath(new URL('..', import.meta.url)),
    },
    bundle: true,
    format: 'esm',
    write: false,
    target: 'es2022',
  });
  client = result.outputFiles[0].text;
});

test.beforeEach(async ({ page }) => {
  await page.route('**/router-trusted-types.html', route => route.fulfill({
    contentType: 'text/html',
    headers: {
      'Content-Security-Policy': "script-src 'nonce-document-nonce'; style-src 'none'; require-trusted-types-for 'script'; trusted-types router-compiled",
    },
    body: '<!doctype html><p id="current">Current route</p><script nonce="document-nonce" type="module" src="/router-trusted-types.js"></script>',
  }));
  await page.route('**/router-trusted-types.js', route => route.fulfill({
    contentType: 'text/javascript',
    body: client,
  }));
});

for (const mode of ['templates', 'json', 'ndjson'] as const) {
  test(`enforced Trusted Types rejects ${mode} source before response publication`, async ({ page }) => {
    await page.route(mode === 'templates' ? '**/_webui/templates?*' : '**/next', route => {
      const payload = {
        path: '/next',
        chain: [{ component: 'untrusted-card', path: '/next' }],
        templates: { 'untrusted-card': { h: '<p>Next</p>' } },
        templateFunctions: { 'untrusted-card': '[(window.sourceExecuted=true,function(){return true})]' },
        inventory: 'after',
        css: ['/untrusted.css'],
      };
      return route.fulfill({
        contentType: mode === 'ndjson' ? 'application/x-ndjson' : 'application/json',
        body: JSON.stringify(payload) + (mode === 'ndjson' ? '\n' : ''),
      });
    });
    await page.goto('/router-trusted-types.html');
    await page.waitForFunction(() => window.routerTrustTest !== undefined);
    const outcome = await page.evaluate(async mode => {
      const runtimeBefore = JSON.stringify(window.__webui);
      const scriptsBefore = document.scripts.length;
      let message = '';
      try {
        if (mode === 'templates') await window.routerTrustTest.ensureLoaded('untrusted-card');
        else await window.routerTrustTest.fetchPartial('/next');
      } catch (error) {
        if (!(error instanceof Error)) throw error;
        message = error.message;
      }
      const factory = (window as Window & {
        trustedTypes?: { defaultPolicy: unknown };
      }).trustedTypes;
      if (!factory) throw new Error('This regression requires native Trusted Types enforcement.');
      return {
        message,
        policyName: window.__webuiTrustedTypesPolicyName,
        bridge: Object.hasOwn(window, '__webuiTrustedTemplates'),
        defaultPolicy: factory.defaultPolicy,
        sourceExecuted: Object.hasOwn(window, 'sourceExecuted'),
        registryUnchanged: JSON.stringify(window.__webui) === runtimeBefore,
        scriptsUnchanged: document.scripts.length === scriptsBefore,
        styleAdded: document.querySelector('link[href="/untrusted.css"]') !== null,
      };
    }, mode);
    expect(outcome).toEqual({
      message: expect.stringMatching(/Trusted Types.*condition-source.*full document navigation/),
      policyName: 'router-compiled',
      bridge: false,
      defaultPolicy: null,
      sourceExecuted: false,
      registryUnchanged: true,
      scriptsUnchanged: true,
      styleAdded: false,
    });
    await expect(page.locator('#current')).toHaveText('Current route');
  });
}
