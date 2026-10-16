// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { build, fixture, removeFixtureRoot, write } from './native-fixture.js';

for (const mode of ['all', 'content']) {
  test(`Markdown-local state survives SSR hydration and interaction in ${mode} mode`, async t => {
    const f = fixture();
    t.after(() => removeFixtureRoot(f.root));
    write(f.configFile, JSON.stringify({ ...f.config, regions: {} }));
    for (const [name, count] of [['first', 5], ['second', 9]] as const) {
      write(path.join(f.site, 'content', `${name}.md`), `---
title: ${name} example
stateFile: ../target/webui-press-state/${name}.json
state:
  count: ${count}
  catalogMessage: Local ${name}
  example:
    label: Save ${name}
    count: 2
    enabled: false
    optional: null
    items: [one, two]
---

# ${name} example

<test-catalog-button label="Preview" :count="{{count}}"></test-catalog-button>
<test-catalog-text></test-catalog-text>
<state-preview :example="{{example}}"></state-preview>
<inspector-preview :inspector="{{inspector}}"></inspector-preview>

<script type="module" bundle>
import '@fixture/catalog/button.js';
</script>
`);
      write(path.join(f.site, 'target/webui-press-state', `${name}.json`), JSON.stringify({
        inspector: {
          example: { label: `Included ${name}`, count: 4, items: ['alpha', 'beta'] },
        },
      }));
    }

    const component = path.join(f.site, 'content/local/state-preview/state-preview');
    write(`${component}.html`,
      '<button @click="{increment()}">{{example.label}} {{example.count}}</button>' +
      '<ul><for each="item in example.items"><li>{{item}}</li></for></ul>');
    write(`${component}.ts`, `
      import { WebUIElement, observable } from '@microsoft/webui-framework';
      export class StatePreview extends WebUIElement {
        @observable example = { count: 0 };
        increment() {
          this.example = { ...this.example, count: this.example.count + 1 };
        }
      }
      StatePreview.define('state-preview');
    `);
    const inspector = path.join(f.site, 'content/local/inspector-preview/inspector-preview');
    write(`${inspector}.html`,
      '<button @click="{increment()}">{{inspector.example.label}} {{inspector.example.count}}</button>' +
      '<ul><for each="item in inspector.example.items"><li>{{item}}</li></for></ul>');
    write(`${inspector}.ts`, `
      import { WebUIElement, observable } from '@microsoft/webui-framework';
      export class InspectorPreview extends WebUIElement {
        @observable inspector = { example: { count: 0, items: [] } };
        increment() {
          this.inspector = {
            ...this.inspector,
            example: { ...this.inspector.example, count: this.inspector.example.count + 1 },
          };
        }
      }
      InspectorPreview.define('inspector-preview');
    `);
    build(f.site, `--show=${mode}`);
    const root = path.join(f.site, 'dist');
    const browser = await chromium.launch({ headless: true });
    t.after(() => browser.close());
    const context = await browser.newContext();
    await context.route('http://press.test/**', async route => {
      const url = new URL(route.request().url());
      const relative = decodeURIComponent(url.pathname).replace(/^\/fixture\//, '');
      let file = path.resolve(root, relative);
      assert.ok(file.startsWith(root + path.sep), 'request escaped the built site');
      if (fs.statSync(file).isDirectory()) file = path.join(file, 'index.html');
      const contentType: Record<string, string> = {
        '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css',
        '.svg': 'image/svg+xml', '.json': 'application/json',
      };
      await route.fulfill({ path: file, contentType: contentType[path.extname(file)] });
    });
    const page = await context.newPage();
    const errors: string[] = [];
    page.on('pageerror', error => errors.push(error.message));
    page.on('console', message => {
      if (message.type() === 'warning' || message.type() === 'error') errors.push(message.text());
    });
    for (const [name, initial] of [['first', 5], ['second', 9], ['doc', 2], ['first', 5]] as const) {
      let releaseScripts: () => void = () => {};
      const scriptsReady = new Promise<void>(resolve => { releaseScripts = resolve; });
      await page.route('**/assets/*.js*', async route => {
        await scriptsReady;
        await route.fallback();
      });
      await page.goto(`http://press.test/fixture/${name}/`, { waitUntil: 'commit' });
      const button = await page.getByRole('button', { name: `Preview ${initial}` }).elementHandle();
      assert.ok(button, 'local initial state must render before scripts run');
      const includedButton = name === 'doc' ? undefined :
        await page.getByRole('button', { name: `Included ${name} 4` }).elementHandle();
      const data = await page.locator('#webui-data').textContent();
      assert.ok(data);
      const bootstrap = JSON.parse(data) as {
        state: { count: number; example: unknown; inspector?: unknown };
      };
      assert.equal(bootstrap.state.count, initial);
      if (name === 'doc') assert.equal(bootstrap.state.example, undefined);
      releaseScripts();
      await expect.poll(async () => {
        assert.deepEqual(errors, []);
        return page.locator('test-catalog-button').evaluate(host =>
          (host as HTMLElement & { $ready?: boolean }).$ready);
      }).toBe(true);
      assert.ok(await button.evaluate(el =>
        el === document.querySelector('test-catalog-button')?.shadowRoot?.querySelector('button')),
      'hydration must preserve the SSR node');
      const count = await page.locator('test-catalog-button').evaluate(host =>
        (host as HTMLElement & { count: number }).count);
      assert.equal(count, initial);
      if (name !== 'doc') {
        await page.waitForFunction(() =>
          (document.querySelector('state-preview') as HTMLElement & { $ready?: boolean })?.$ready);
        const example = await page.locator('state-preview').evaluate(host =>
          (host as HTMLElement & { example: unknown }).example);
        assert.deepEqual(example, {
          label: `Save ${name}`, count: 2, enabled: false, optional: null, items: ['one', 'two'],
        });
        await expect(page.locator('test-catalog-text')).toHaveText(`Local ${name}`);
        assert.deepEqual(bootstrap.state.example, example);
        await page.getByRole('button', { name: `Save ${name} 2` }).click();
        await expect(page.getByRole('button', { name: `Save ${name} 3` })).toBeVisible();
        await page.waitForFunction(() =>
          (document.querySelector('inspector-preview') as HTMLElement & { $ready?: boolean })?.$ready);
        const inspector = await page.locator('inspector-preview').evaluate(host =>
          (host as HTMLElement & { inspector: unknown }).inspector);
        assert.deepEqual(inspector, {
          example: { label: `Included ${name}`, count: 4, items: ['alpha', 'beta'] },
        });
        assert.deepEqual(bootstrap.state.inspector, inspector);
        assert.ok(includedButton);
        assert.ok(await includedButton.evaluate(el =>
          el === document.querySelector('inspector-preview')?.shadowRoot?.querySelector('button')));
        await page.getByRole('button', { name: `Included ${name} 4` }).click();
        await expect(page.getByRole('button', { name: `Included ${name} 5` })).toBeVisible();
      }

      await button.click();
      await expect(page.getByRole('button', { name: `Preview ${initial + 1}` })).toBeVisible();
      if (name === 'first' && process.env.WEBUI_PRESS_SCREENSHOTS) {
        fs.mkdirSync(process.env.WEBUI_PRESS_SCREENSHOTS, { recursive: true });
        for (const width of [1440, 390]) {
          await page.setViewportSize({ width, height: 1000 });
          await page.screenshot({
            path: path.join(process.env.WEBUI_PRESS_SCREENSHOTS, `markdown-state-${mode}-${width}.png`),
            fullPage: true,
          });
        }
      }
      await page.unroute('**/assets/*.js*');
    }
    assert.deepEqual(errors, []);
  });
}
