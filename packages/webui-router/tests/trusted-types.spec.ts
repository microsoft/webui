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
        import { WebUIElement, registerTemplateData } from '../webui-framework/src/index.ts';
        import { WebUIRouter } from './src/router.ts';
        window.__webui = { inventory: 'before', nonce: 'document-nonce' };
        registerTemplateData({ 'compiled-start': { h: '<p>Compiled start</p>', sd: 1 } });
        class CompiledStart extends WebUIElement {}
        CompiledStart.define('compiled-start');
        document.body.appendChild(document.createElement('compiled-start'));
        window.routerTrustTest = new WebUIRouter();
      `,
      resolveDir: fileURLToPath(new URL('..', import.meta.url)),
    },
    bundle: true,
    format: 'esm',
    write: false,
    target: 'es2022',
    define: { __WEBUI_DEV__: 'false' },
  });
  client = result.outputFiles[0].text;
});

for (const enforced of [true, false]) {
  test.describe(enforced ? 'enforced Trusted Types' : 'Trusted Types without enforcement', () => {
    test.beforeEach(async ({ page }) => {
      await page.route('**/router-trusted-types.html', route => route.fulfill({
        contentType: 'text/html',
        headers: {
          'Content-Security-Policy': "script-src 'nonce-document-nonce'; style-src 'none'; trusted-types webui"
            + (enforced ? "; require-trusted-types-for 'script'" : ''),
        },
        body: '<!doctype html><p id="current">Current route</p><script nonce="document-nonce" type="module" src="/router-trusted-types.js"></script>',
      }));
      await page.route('**/router-trusted-types.js', route => route.fulfill({
        contentType: 'text/javascript',
        body: client,
      }));
    });

    for (const mode of ['templates', 'json', 'ndjson', 'fast'] as const) {
      test(`${mode} registration follows native enforcement, not policy creation`, async ({ page }) => {
        await page.route(mode === 'templates' || mode === 'fast' ? '**/_webui/templates?*' : '**/next', route => {
          const payload = {
            path: '/next',
            chain: [{ component: 'untrusted-card', path: '/next' }],
            templates: {
              'untrusted-card': mode === 'fast' ? '<f-template>Next</f-template>' : { h: '<p>Next</p>' },
            },
            templateFunctions: mode === 'fast'
              ? {}
              : { 'untrusted-card': '[(window.sourceExecuted=true,function(){return true})]' },
            inventory: 'after',
            css: ['/untrusted.css'],
          };
          return route.fulfill({
            contentType: mode === 'ndjson' ? 'application/x-ndjson' : 'application/json',
            body: JSON.stringify(payload) + (mode === 'ndjson' ? '\n' : ''),
          });
        });
        await page.goto('/router-trusted-types.html');
        await expect(page.locator('compiled-start')).toHaveText('Compiled start');
        const outcome = await page.evaluate(async mode => {
          const runtimeBefore = JSON.stringify(window.__webui);
          const scriptsBefore = document.scripts.length;
          let message = '';
          try {
            if (mode === 'templates' || mode === 'fast') await window.routerTrustTest.ensureLoaded('untrusted-card');
            else await window.routerTrustTest.fetchPartial('/next');
          } catch (error) {
            if (!(error instanceof Error)) throw error;
            message = error.message;
          }
          const factory = (window as Window & {
            trustedTypes?: { defaultPolicy: unknown };
          }).trustedTypes;
          if (!factory) throw new Error('This regression requires native Trusted Types support.');
          return {
            message,
            marker: Object.hasOwn(window, '__webuiTrustedTypesPolicyName'),
            bridge: Object.hasOwn(window, '__webuiTrustedTemplates'),
            defaultPolicy: factory.defaultPolicy,
            sourceExecuted: Object.hasOwn(window, 'sourceExecuted'),
            registryUnchanged: JSON.stringify(window.__webui) === runtimeBefore,
            scriptsUnchanged: document.scripts.length === scriptsBefore,
            styleAdded: document.querySelector('link[href="/untrusted.css"]') !== null,
            fastAdded: document.querySelector('f-template') !== null,
          };
        }, mode);
        expect(outcome).toEqual({
          message: enforced
            ? expect.stringMatching(mode === 'fast'
              ? /Trusted Types.*FAST\/string templates.*full document navigation/
              : /Trusted Types.*condition-source.*full document navigation/)
            : '',
          marker: false,
          bridge: false,
          defaultPolicy: null,
          sourceExecuted: !enforced && mode !== 'fast',
          registryUnchanged: enforced,
          scriptsUnchanged: true,
          styleAdded: !enforced && (mode === 'json' || mode === 'ndjson'),
          fastAdded: !enforced && mode === 'fast',
        });
        await expect(page.locator('#current')).toHaveText('Current route');
      });
    }
  });
}
