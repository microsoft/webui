// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { chromium } from '@playwright/test';

const port = Number(process.argv[2]);
const timeoutMs = Number(process.argv[3] ?? 30_000);
const deadline = Date.now() + timeoutMs;
let browser;

function sleep(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
}

try {
  while (!browser && Date.now() < deadline) {
    try {
      browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
    } catch {
      await sleep(25);
    }
  }
  if (!browser) throw new Error('CDP endpoint did not become available');

  let page;
  while (!page && Date.now() < deadline) {
    const pages = browser.contexts().flatMap(context => context.pages());
    page = pages.find(candidate => candidate.url().startsWith('https://app.webui.localhost/')) ?? pages[0];
    if (!page) await sleep(25);
  }
  if (!page) throw new Error('host page did not become available');

  const remaining = Math.max(1, deadline - Date.now());
  await page.waitForFunction(
    () => document.querySelector('cb-app')?.$ready === true,
    undefined,
    { timeout: remaining },
  );
  await page.locator('cb-page-dashboard .page-title').waitFor({
    state: 'visible',
    timeout: Math.max(1, deadline - Date.now()),
  });

  const result = await page.evaluate(async () => {
    await document.fonts.ready;
    await new Promise(resolve => {
      requestAnimationFrame(() => requestAnimationFrame(resolve));
    });
    const resources = [];
    const queryDeep = (root, selector) => {
      const direct = root.querySelector(selector);
      if (direct) return direct;
      for (const element of root.querySelectorAll('*')) {
        if (element.shadowRoot) {
          const nested = queryDeep(element.shadowRoot, selector);
          if (nested) return nested;
        }
      }
      return null;
    };
    const collectResources = root => {
      for (const element of root.querySelectorAll('*')) {
        if (element.matches('[data-webui-resource]')) {
          resources.push(element.dataset.webuiResource ?? '');
        }
        if (element.shadowRoot) collectResources(element.shadowRoot);
      }
    };
    collectResources(document);
    const hydration = performance.getEntriesByName('webui:hydrate:total', 'measure')[0];
    const title = document.querySelector('title')?.textContent ?? '';
    const dashboard = queryDeep(document, 'cb-page-dashboard');
    const pageTitle = queryDeep(dashboard?.shadowRoot ?? document, '.page-title')
      ?.textContent?.trim() ?? '';
    const app = queryDeep(document, 'cb-app');
    const bodyStyle = getComputedStyle(document.body);
    const bodyBackground = bodyStyle.backgroundColor;
    const dashboardStyle = getComputedStyle(dashboard);
    const sidebar = queryDeep(document, 'cb-sidebar');
    const sidebarStyle = getComputedStyle(sidebar);
    return {
      readyEpochMs: performance.timeOrigin + performance.now(),
      hydrationMs: hydration?.duration ?? null,
      title,
      pageTitle,
      appReady: app?.$ready === true,
      fontsReady: document.fonts.status === 'loaded',
      completedPaintFrames: 2,
      innerWidth: window.innerWidth,
      innerHeight: window.innerHeight,
      devicePixelRatio: window.devicePixelRatio,
      stylesheetCount: document.querySelectorAll('link[rel="stylesheet"],style').length,
      resourceCount: resources.length,
      bodyBackground,
      bodyFontFamily: bodyStyle.fontFamily,
      dashboardDisplay: dashboardStyle.display,
      sidebarWidth: sidebarStyle.width,
      dashboardText: queryDeep(dashboard?.shadowRoot ?? document, '.section-title')
        ?.textContent?.trim() ?? '',
    };
  });

  if (
    result.pageTitle !== 'Dashboard' ||
    !result.appReady ||
    result.hydrationMs === null ||
    !result.fontsReady ||
    result.completedPaintFrames !== 2 ||
    result.innerWidth !== 1200 ||
    result.innerHeight !== 800 ||
    !Number.isFinite(result.devicePixelRatio) ||
    result.stylesheetCount === 0 ||
    result.resourceCount === 0 ||
    result.bodyBackground === 'rgba(0, 0, 0, 0)' ||
    !result.bodyFontFamily.includes('Segoe UI') ||
    result.dashboardDisplay !== 'block' ||
    result.sidebarWidth !== '260px' ||
    result.dashboardText !== 'Recent Contacts'
  ) {
    throw new Error(`Dashboard readiness contract failed: ${JSON.stringify(result)}`);
  }
  process.stdout.write(`${JSON.stringify({ ok: true, ...result })}\n`);
} catch (error) {
  process.stderr.write(`${error instanceof Error ? error.stack : String(error)}\n`);
  process.exitCode = 1;
} finally {
  await browser?.close();
}
