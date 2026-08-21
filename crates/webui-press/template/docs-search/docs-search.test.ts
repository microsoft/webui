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

test('nested documentation components hydrate without page errors', async (t) => {
  const server = await startDocsServer();
  t.after(() => server.close());

  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());

  const page = await browser.newPage();
  const pageErrors: string[] = [];
  page.on('pageerror', (error) => pageErrors.push(error.message));

  await page.goto(`${server.origin}/webui/`, { waitUntil: 'networkidle' });
  await waitForDocsSearch(page);
  await page.goto(`${server.origin}/webui/guide/`, {
    waitUntil: 'networkidle',
  });
  await waitForDocsSearch(page);
  await page.goto(`${server.origin}/webui/#%E0%A4`, {
    waitUntil: 'networkidle',
  });
  await waitForDocsSearch(page);

  assert.deepEqual(pageErrors, []);
});

test('Playground initializes after navigation from the docs home page', async (t) => {
  const server = await startDocsServer();
  t.after(() => server.close());

  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());

  const page = await browser.newPage();
  const pageErrors: string[] = [];
  page.on('pageerror', (error) => pageErrors.push(error.message));

  await page.goto(`${server.origin}/webui/`, { waitUntil: 'networkidle' });
  await waitForDocsSearch(page);
  const navigationState = await page
    .locator('docs-site-navigation')
    .evaluate((navigation) => ({
      count: navigation.navigation?.length ?? 0,
      playgroundFullLayout:
        navigation.navigation?.find((link) => link.text === 'Playground')
          ?.fullLayout ?? false,
    }));
  assert.ok(navigationState.count > 0);
  assert.equal(navigationState.playgroundFullLayout, true);
  const playgroundLink = page
    .locator('docs-site-navigation')
    .locator('a')
    .filter({ hasText: 'Playground' })
    .first();

  await Promise.all([
    page.waitForURL('**/webui/playground/**'),
    playgroundLink.click(),
  ]);
  await page.waitForFunction(
    () =>
      !(document as Document & { activeViewTransition?: unknown })
        .activeViewTransition,
  );
  await page.waitForFunction(() => {
    const playground = document.querySelector('docs-playground');
    return Boolean(
      customElements.get('docs-playground') &&
        playground?.shadowRoot?.querySelector('.pg'),
    );
  });

  const playground = await page.locator('docs-playground').evaluate((host) => {
    const rect = host.getBoundingClientRect();
    return {
      width: rect.width,
      height: rect.height,
      editor: Boolean(host.shadowRoot?.querySelector('.editor-wrap .cm-editor')),
    };
  });

  assert.ok(playground.width > 0);
  assert.ok(playground.height > 0);
  assert.equal(playground.editor, true);

  const guideLink = page
    .locator('docs-site-navigation')
    .locator('a')
    .filter({ hasText: 'Guide' })
    .first();
  await Promise.all([
    page.waitForURL('**/webui/guide/**'),
    guideLink.click(),
  ]);
  await page.waitForFunction(
    () =>
      !(document as Document & { activeViewTransition?: unknown })
        .activeViewTransition,
  );
  assert.ok(
    (await page.locator('.doc-content h1').innerText()).startsWith(
      'What is WebUI Framework?',
    ),
  );
  assert.deepEqual(pageErrors, []);
});

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
    const root = el.shadowRoot;
    if (!root) throw new Error('docs-search ShadowRoot missing');
    const result = root.querySelector('.result');
    if (!result) throw new Error('docs-search result missing');
    const title = result.querySelector('.result-title');
    if (!title) throw new Error('docs-search result title missing');
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
    const input = el.shadowRoot?.querySelector<HTMLInputElement>('input');
    if (!input) throw new Error('docs-search input missing');
    input.value = 'wasm bundles';
    el.onInput();
    await new Promise((resolve) => setTimeout(resolve, 0));
  });

  const separator = await page.locator('docs-search').evaluate((el) => {
    const root = el.shadowRoot;
    if (!root) throw new Error('docs-search ShadowRoot missing');
    const results = [...root.querySelectorAll('.result')];
    const hrefs = results.map((result) => result.getAttribute('href'));
    const item = results.find((result) =>
      result.getAttribute('href').includes('#building-the-wasm-bundles'),
    );
    if (!item) {
      return {
        found: false,
        hrefs,
        text: '',
        marginLeft: 0,
        marginRight: 0,
        normalizedTitle: '',
      };
    }
    const sep = item.querySelector('.result-separator');
    const style = getComputedStyle(sep);
    return {
      found: true,
      hrefs,
      text: sep.textContent,
      marginLeft: Number.parseFloat(style.marginLeft),
      marginRight: Number.parseFloat(style.marginRight),
      normalizedTitle: item
        .querySelector('.result-title')
        .textContent.replace(/\s+/g, ''),
    };
  });

  assert.ok(
    separator.found,
    `no heading result linking to #building-the-wasm-bundles; got [${separator.hrefs.join(', ')}]`,
  );
  assert.equal(separator.text, '>');
  assert.equal(
    separator.normalizedTitle,
    'WebUIWebAssembly>BuildingtheWASMbundles',
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

  const light = await page
    .locator('docs-site-navigation')
    .evaluate((navigation) => {
      const main = document.querySelector('.main-content');
      const search = navigation.shadowRoot
        ?.querySelector('docs-search')
        ?.shadowRoot?.querySelector('.trigger');
      const theme = navigation.shadowRoot
        ?.querySelector('docs-theme-toggle')
        ?.shadowRoot?.querySelector('button');
      const style = getComputedStyle(main);
      return {
        thumb: style.getPropertyValue('--docs-scrollbar-thumb').trim(),
        track: style.getPropertyValue('--docs-scrollbar-track').trim(),
        searchHeight: search?.getBoundingClientRect().height ?? 0,
        themeHeight: theme?.getBoundingClientRect().height ?? 0,
      };
    });

  await page.evaluate(() => {
    document.documentElement.setAttribute('data-theme', 'dark');
  });

  await page.locator('docs-search').evaluate(async (el) => {
    el.openSearch();
    await new Promise((resolve) => setTimeout(resolve, 300));
    const input = el.shadowRoot?.querySelector<HTMLInputElement>('input');
    if (!input) throw new Error('docs-search input missing');
    input.value = 'asse';
    el.onInput();
    await new Promise((resolve) => setTimeout(resolve, 0));
  });

  const dark = await page.locator('docs-search').evaluate((el) => {
    const root = el.shadowRoot;
    if (!root) throw new Error('docs-search ShadowRoot missing');
    const mark = root.querySelector('mark');
    if (!mark) throw new Error('docs-search highlight missing');
    const hostStyle = getComputedStyle(el);
    return {
      thumb: hostStyle.getPropertyValue('--docs-scrollbar-thumb').trim(),
      track: hostStyle.getPropertyValue('--docs-scrollbar-track').trim(),
      markBg: getComputedStyle(mark).backgroundColor,
    };
  });

  assert.equal(light.thumb, '#9ca3af');
  assert.equal(light.track, '#f3f4f6');
  assert.equal(light.searchHeight, 36);
  assert.equal(light.themeHeight, light.searchHeight);
  assert.equal(dark.thumb, '#4b5563');
  assert.equal(dark.track, '#111827');
  assert.equal(dark.markBg, 'rgb(183, 121, 31)');
});

test('search highlighting preserves spaces between title segments', async (t) => {
  const server = await startDocsServer();
  t.after(() => server.close());

  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());

  const page = await browser.newPage();
  await page.goto(`${server.origin}/webui/guide/concepts/directives/signals/`, {
    waitUntil: 'networkidle',
  });
  await waitForDocsSearch(page);

  const title = await page.locator('docs-search').evaluate(async (el) => {
    el.openSearch();
    await new Promise((resolve) => setTimeout(resolve, 300));
    const input = el.shadowRoot.querySelector('input');
    input.value = 'signal';
    el.onInput();
    await new Promise((resolve) => setTimeout(resolve, 0));
    const result = [...el.shadowRoot.querySelectorAll('.result')].find((item) =>
      item.getAttribute('href')?.endsWith('/guide/concepts/directives/signals'),
    );
    return (result?.querySelector('.result-title') as HTMLElement | null)
      ?.innerText ?? '';
  });

  assert.equal(title.trim().replace(/\s+/g, ' '), 'Signal Directives');
});

test('mobile navigation opens everywhere and restores focus on escape', async (t) => {
  const server = await startDocsServer();
  t.after(() => server.close());

  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());

  const page = await browser.newPage({ viewport: { width: 390, height: 844 } });
  await page.goto(`${server.origin}/webui/`, { waitUntil: 'networkidle' });

  const component = page.locator('docs-site-navigation');
  const trigger = component.locator('.mobile-menu-btn');
  const navigation = component.locator('.mobile-navigation');
  const header = await page.evaluate(() => {
    const nav = document.querySelector('.nav-bar');
    const menu = document
      .querySelector('docs-site-navigation')
      ?.shadowRoot?.querySelector('.mobile-menu-btn');
    const navRect = nav?.getBoundingClientRect();
    const menuRect = menu?.getBoundingClientRect();
    return {
      scrollWidth: nav?.scrollWidth ?? 0,
      clientWidth: nav?.clientWidth ?? 0,
      menuRight: menuRect?.right ?? 0,
      navRight: navRect?.right ?? 0,
    };
  });
  assert.ok(header.scrollWidth <= header.clientWidth);
  assert.ok(header.menuRight <= header.navRight);
  assert.equal(await navigation.isHidden(), true);

  await trigger.click();
  assert.equal(await trigger.getAttribute('aria-expanded'), 'true');
  assert.equal(await navigation.isVisible(), true);
  assert.equal(
    await component.evaluate(
      (element) =>
        element.shadowRoot?.activeElement?.classList.contains(
          'mobile-navigation-close',
        ) ?? false,
    ),
    true,
  );

  await page.keyboard.press('Escape');
  await page.waitForFunction(() => {
    const navigation = document.querySelector('docs-site-navigation');
    const trigger = navigation?.shadowRoot?.querySelector('.mobile-menu-btn');
    return trigger?.getAttribute('aria-expanded') === 'false';
  });
  assert.equal(await trigger.getAttribute('aria-expanded'), 'false');
  assert.equal(await navigation.isHidden(), true);
  assert.equal(
    await component.evaluate(
      (element) =>
        element.shadowRoot?.activeElement?.classList.contains(
          'mobile-menu-btn',
        ) ?? false,
    ),
    true,
  );
});

test('documentation shell preserves semantic hierarchy and reading measure', async (t) => {
  const server = await startDocsServer();
  t.after(() => server.close());

  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());

  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  await page.goto(`${server.origin}/webui/`, { waitUntil: 'networkidle' });
  const homeHeadings = await page
    .locator('#main-content h1, #main-content h2, #main-content h3')
    .evaluateAll((nodes) => nodes.map((node) => node.tagName));
  assert.equal(homeHeadings[0], 'H1');
  assert.ok(homeHeadings.slice(1).every((tag) => tag === 'H2'));

  await page.goto(
    `${server.origin}/webui/guide/concepts/directives/signals/`,
    { waitUntil: 'networkidle' },
  );
  const shell = await page.evaluate(() => {
    const sidebar = document.querySelector(
      '.sidebar docs-sidebar-navigation',
    );
    const sidebarRoot = sidebar?.shadowRoot;
    const sidebarTitles = [
      ...(sidebarRoot?.querySelectorAll('.sidebar-title') ?? []),
    ];
    const anchor = document.querySelector('.doc-content .header-anchor');
    const article = document.querySelector('.doc-content');
    const main = document.querySelector('.main-content');
    const templateDetails = [
      ...(sidebarRoot?.querySelectorAll<HTMLDetailsElement>(
        '.sidebar-branch',
      ) ?? []),
    ].find((details) =>
      details
        .querySelector('.sidebar-summary')
        ?.textContent?.includes('Template Syntax'),
    );
    const cliDetails = [
      ...(sidebarRoot?.querySelectorAll<HTMLDetailsElement>(
        '.sidebar-branch',
      ) ?? []),
    ].find((details) =>
      details
        .querySelector('.sidebar-summary')
        ?.textContent?.includes('CLI Reference'),
    );
    return {
      sidebarTitleTags: sidebarTitles.map((title) => title.tagName),
      anchorLabel: anchor?.getAttribute('aria-label') ?? '',
      articleWidth: article?.getBoundingClientRect().width ?? 0,
      mainWidth: main?.getBoundingClientRect().width ?? 0,
      templateExpanded: templateDetails?.open,
      cliExpanded: cliDetails?.open,
      customNavigationDefined: Boolean(
        customElements.get('docs-site-navigation'),
      ),
      nativeDisclosureCount:
        sidebarRoot?.querySelectorAll('details.sidebar-branch').length ?? 0,
    };
  });

  assert.ok(shell.sidebarTitleTags.every((tag) => tag === 'DIV'));
  assert.ok(shell.anchorLabel.startsWith('Link to '));
  assert.ok(shell.articleWidth > 0 && shell.articleWidth <= 800);
  assert.ok(shell.articleWidth < shell.mainWidth);
  assert.equal(shell.templateExpanded, true);
  assert.equal(shell.cliExpanded, false);
  assert.equal(shell.customNavigationDefined, true);
  assert.ok(shell.nativeDisclosureCount > 0);

  const templateDisclosure = page
    .locator('.sidebar docs-sidebar-navigation')
    .locator('details.sidebar-branch')
    .filter({ hasText: 'Template Syntax' });
  await templateDisclosure.locator('summary').click();
  assert.equal(await templateDisclosure.getAttribute('open'), null);
});

test('playground runtime failures offer recovery without exposing raw details', async (t) => {
  const server = await startDocsServer();
  t.after(() => server.close());

  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());

  const page = await browser.newPage({ viewport: { width: 390, height: 844 } });
  let loadAttempts = 0;
  await page.route('**/wasm/all/webui_wasm_all.js*', async (route) => {
    loadAttempts += 1;
    await route.abort();
  });
  await page.goto(`${server.origin}/webui/playground/`, {
    waitUntil: 'networkidle',
  });
  await page.waitForFunction(() =>
    document
      .querySelector('docs-playground')
      ?.shadowRoot?.querySelector('.error-recovery'),
  );

  const failure = await page.locator('docs-playground').evaluate((el) => {
    const root = el.shadowRoot;
    const panel = root.querySelector('.error-panel');
    const addButton = root.querySelector('.tab-add-btn');
    const activeTab = root.querySelector('.tab[active]');
    const activeTabButton = activeTab?.querySelector('.tab-select');
    const activeTabName = activeTabButton?.querySelector('.tab-name');
    const closeButton = root.querySelector('.tab-close-btn');
    const hostRect = el.getBoundingClientRect();
    const addRect = addButton?.getBoundingClientRect();
    return {
      title: root.querySelector('.error-panel-title')?.textContent?.trim(),
      retry: root.querySelector('.error-retry-btn')?.textContent?.trim(),
      help: root.querySelector('.error-help-text')?.textContent?.trim(),
      expanded: panel?.hasAttribute('data-expanded'),
      addWidth: addRect?.width ?? 0,
      addInsideHost: addRect ? addRect.right <= hostRect.right : false,
      tabGroupRole: root.querySelector('.tab-strip')?.getAttribute('role'),
      activeTabTag: activeTabButton?.tagName,
      activeTabPressed: activeTabButton?.getAttribute('aria-pressed'),
      activeTabNameVisible: activeTabName
        ? activeTabName.scrollWidth <= activeTabName.clientWidth
        : false,
      addLabel: addButton?.getAttribute('aria-label'),
      closeLabel: closeButton?.getAttribute('aria-label') ?? '',
      editorLabel: root.querySelector('.editor-wrap')?.getAttribute('aria-label'),
      previewTitle: root.querySelector('.preview-frame')?.getAttribute('title'),
    };
  });

  assert.equal(failure.title, "Preview couldn't load.");
  assert.equal(failure.retry, 'Retry preview');
  assert.ok(failure.help?.includes('cargo xtask build-wasm'));
  assert.equal(failure.expanded, false);
  assert.ok(failure.addWidth >= 44);
  assert.equal(failure.addInsideHost, true);
  assert.equal(failure.tabGroupRole, 'group');
  assert.equal(failure.activeTabTag, 'BUTTON');
  assert.equal(failure.activeTabPressed, 'true');
  assert.equal(failure.activeTabNameVisible, true);
  assert.equal(failure.addLabel, 'New file');
  assert.ok(failure.closeLabel.startsWith('Close '));
  assert.equal(failure.editorLabel, 'Code editor');
  assert.equal(failure.previewTitle, 'Rendered preview');

  await page.locator('docs-playground').locator('.error-retry-btn').click();
  await page.waitForFunction(() => {
    const host = document.querySelector('docs-playground');
    return host?.shadowRoot?.querySelector('.error-retry-btn');
  });
  assert.ok(loadAttempts >= 2, `loadAttempts=${loadAttempts}`);
});

async function waitForDocsSearch(page: Page): Promise<void> {
  await page.waitForFunction(() => {
    const el = document
      .querySelector('docs-site-navigation')
      ?.shadowRoot?.querySelector('docs-search');
    const root = el?.shadowRoot;
    return (
      root &&
      customElements.get('docs-search') &&
      root.querySelector('input')
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
