// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { chromium } from '@playwright/test';
import type { Page } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import http from 'node:http';
import type { AddressInfo } from 'node:net';
import path from 'node:path';
import test from 'node:test';

const DIST = resolveDocsDist();

const MIME_TYPES = new Map([
  ['.css', 'text/css'],
  ['.html', 'text/html'],
  ['.js', 'text/javascript'],
  ['.json', 'application/json'],
  ['.svg', 'image/svg+xml'],
  ['.wasm', 'application/wasm'],
]);

function resolveDocsDist(): string {
  let current = process.cwd();
  loop: while (true) {
    const candidate = path.join(current, 'docs', 'dist');
    if (fs.existsSync(candidate)) {
      return candidate;
    }
    const parent = path.dirname(current);
    if (parent === current) {
      break loop;
    }
    current = parent;
  }
  return path.resolve(process.cwd(), 'docs', 'dist');
}

test('search results do not retain stale title segments while typing', async (t) => {
  const server = await startDocsServer();
  t.after(() => server.close());

  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());

  const page = await browser.newPage();
  await page.goto(`${server.origin}/webui/guide/integrations/wasm/`, {
    waitUntil: 'networkidle',
  });
  await waitForDocsSearch(page);

  await page.locator('docs-search').evaluate(async (el) => {
    el.openSearch();
    await new Promise((resolve) => setTimeout(resolve, 300));
  });

  const input = page.locator('docs-search').locator('input');
  await input.type('asse', { delay: 50 });
  await page.waitForTimeout(500);

  const first = await page.locator('docs-search').evaluate((el) => {
    const result = el.shadowRoot.querySelector('.result');
    const title = result.querySelector('.result-title');
    return {
      href: result.getAttribute('href'),
      normalizedTitle: title.textContent.replace(/\s+/g, ''),
      marks: [...title.querySelectorAll('mark')].map((mark) => mark.textContent),
    };
  });

  assert.equal(first.href, '/webui/guide/integrations/wasm');
  assert.equal(first.normalizedTitle, 'WebUIWebAssembly');
  assert.deepEqual(first.marks, ['Asse']);
});

test('heading search results use CSS spacing for breadcrumb separators', async (t) => {
  const server = await startDocsServer();
  t.after(() => server.close());

  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());

  const page = await browser.newPage();
  await page.goto(`${server.origin}/webui/guide/integrations/wasm/`, {
    waitUntil: 'networkidle',
  });
  await waitForDocsSearch(page);

  await page.locator('docs-search').evaluate(async (el) => {
    el.openSearch();
    await new Promise((resolve) => setTimeout(resolve, 300));
    const input = el.shadowRoot.querySelector('input');
    input.value = 'webassembly';
    el.onInput();
    await new Promise((resolve) => setTimeout(resolve, 0));
  });

  const separator = await page.locator('docs-search').evaluate((el) => {
    const item = [...el.shadowRoot.querySelectorAll('.result')].find((result) =>
      result.getAttribute('href').includes('#webassembly'),
    );
    const sep = item.querySelector('.result-separator');
    const style = getComputedStyle(sep);
    return {
      text: sep.textContent,
      marginLeft: Number.parseFloat(style.marginLeft),
      marginRight: Number.parseFloat(style.marginRight),
      normalizedTitle: item
        .querySelector('.result-title')
        .textContent.replace(/\s+/g, ''),
    };
  });

  assert.equal(separator.text, '>');
  assert.equal(
    separator.normalizedTitle,
    'WebUIFramework-AIReference>WebAssembly',
  );
  assert.ok(separator.marginLeft > 0, `marginLeft=${separator.marginLeft}`);
  assert.ok(separator.marginRight > 0, `marginRight=${separator.marginRight}`);
});

test('documentation pages scroll inside main content, not the window', async (t) => {
  const server = await startDocsServer();
  t.after(() => server.close());

  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());

  const page = await browser.newPage();
  await page.goto(`${server.origin}/webui/guide/concepts/components/`, {
    waitUntil: 'networkidle',
  });

  await page.locator('.main-content').hover();
  await page.mouse.wheel(0, 900);
  await page.waitForTimeout(100);

  const scroll = await page.evaluate(() => {
    const main = document.querySelector('.main-content');
    return {
      windowY: window.scrollY,
      documentY: document.scrollingElement.scrollTop,
      mainY: main.scrollTop,
      mainScrollable: main.scrollHeight > main.clientHeight,
    };
  });

  assert.equal(scroll.windowY, 0);
  assert.equal(scroll.documentY, 0);
  assert.equal(scroll.mainScrollable, true);
  assert.ok(scroll.mainY > 0, `mainY=${scroll.mainY}`);
});

test('documentation pages support keyboard and history scrolling', async (t) => {
  const server = await startDocsServer();
  t.after(() => server.close());

  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());

  const page = await browser.newPage();
  await page.goto(`${server.origin}/webui/guide/concepts/components/`, {
    waitUntil: 'networkidle',
  });

  await page.keyboard.press('PageDown');
  await page.waitForTimeout(100);

  const afterPageDown = await page.evaluate(() => ({
    windowY: window.scrollY,
    mainY: document.querySelector('.main-content').scrollTop,
  }));
  assert.equal(afterPageDown.windowY, 0);
  assert.ok(afterPageDown.mainY > 0, `mainY=${afterPageDown.mainY}`);

  await page.goto(`${server.origin}/webui/guide/concepts/interactivity/`, {
    waitUntil: 'networkidle',
  });
  await page.goBack({ waitUntil: 'networkidle' });

  const restored = await page.evaluate(() => ({
    windowY: window.scrollY,
    mainY: document.querySelector('.main-content').scrollTop,
  }));
  assert.equal(restored.windowY, 0);
  assert.ok(restored.mainY > 0, `mainY=${restored.mainY}`);
});

test('scrollbars and search highlights use theme-specific colors', async (t) => {
  const server = await startDocsServer();
  t.after(() => server.close());

  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());

  const page = await browser.newPage();
  await page.goto(`${server.origin}/webui/guide/integrations/wasm/`, {
    waitUntil: 'networkidle',
  });
  await waitForDocsSearch(page);

  const light = await page.evaluate(() => {
    const main = document.querySelector('.main-content');
    const style = getComputedStyle(main);
    return {
      thumb: style.getPropertyValue('--docs-scrollbar-thumb').trim(),
      track: style.getPropertyValue('--docs-scrollbar-track').trim(),
    };
  });

  await page.evaluate(() => {
    document.documentElement.setAttribute('data-theme', 'dark');
  });

  await page.locator('docs-search').evaluate(async (el) => {
    el.openSearch();
    await new Promise((resolve) => setTimeout(resolve, 300));
    const input = el.shadowRoot.querySelector('input');
    input.value = 'asse';
    el.onInput();
    await new Promise((resolve) => setTimeout(resolve, 0));
  });

  const dark = await page.locator('docs-search').evaluate((el) => {
    const mark = el.shadowRoot.querySelector('mark');
    const hostStyle = getComputedStyle(el);
    return {
      thumb: hostStyle.getPropertyValue('--docs-scrollbar-thumb').trim(),
      track: hostStyle.getPropertyValue('--docs-scrollbar-track').trim(),
      markBg: getComputedStyle(mark).backgroundColor,
    };
  });

  assert.equal(light.thumb, '#9ca3af');
  assert.equal(light.track, '#f3f4f6');
  assert.equal(dark.thumb, '#4b5563');
  assert.equal(dark.track, '#111827');
  assert.equal(dark.markBg, 'rgb(183, 121, 31)');
});

async function waitForDocsSearch(page: Page): Promise<void> {
  await page.waitForFunction(() => {
    const el = document.querySelector('docs-search');
    return (
      el &&
      customElements.get('docs-search') &&
      el.shadowRoot &&
      el.shadowRoot.querySelector('input')
    );
  });
}

async function startDocsServer(): Promise<{
  origin: string;
  close: () => Promise<void>;
}> {
  const server = http.createServer((req, res) => {
    const requestUrl = new URL(req.url, 'http://local.test');
    let urlPath = decodeURIComponent(requestUrl.pathname);
    if (urlPath === '/webui') {
      urlPath = '/';
    } else if (urlPath.startsWith('/webui/')) {
      urlPath = urlPath.slice('/webui'.length);
    }

    let filePath = path.join(DIST, urlPath);
    if (!filePath.startsWith(DIST)) {
      res.writeHead(403).end();
      return;
    }
    if (fs.existsSync(filePath) && fs.statSync(filePath).isDirectory()) {
      filePath = path.join(filePath, 'index.html');
    }
    if (!fs.existsSync(filePath)) {
      filePath = path.join(DIST, '404.html');
    }

    res.setHeader(
      'Content-Type',
      MIME_TYPES.get(path.extname(filePath)) || 'application/octet-stream',
    );
    fs.createReadStream(filePath).pipe(res);
  });

  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const address = server.address() as AddressInfo | null;
  assert.notEqual(address, null);
  return {
    origin: `http://127.0.0.1:${address.port}`,
    close: () =>
      new Promise((resolve) => {
        server.closeIdleConnections?.();
        server.closeAllConnections?.();
        server.close(resolve);
      }),
  };
}
