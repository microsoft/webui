// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import test from 'node:test';
import { startWebsite } from './website-fixture.js';

const article = '/webui/learn/interactivity/';

test('the reading scrollbar reaches the window edge without widening the article', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  for (const width of [1440, 1696, 1920, 2560]) {
    const page = await browser.newPage({ javaScriptEnabled: false, viewport: { width, height: 900 } });
    await page.goto(`${server.origin}/webui/start/`);
    const bounds = await page.evaluate(() => {
      const main = document.querySelector<HTMLElement>('.main-content');
      const article = document.querySelector<HTMLElement>('.doc-content');
      const sidebar = document.querySelector<HTMLElement>('.sidebar');
      if (!main || !article || !sidebar) throw new Error('Missing reading layout');
      return {
        right: main.getBoundingClientRect().right,
        articleWidth: article.getBoundingClientRect().width,
        articleLimit: Number.parseFloat(getComputedStyle(article).maxWidth),
        sidebarLeft: sidebar.getBoundingClientRect().left,
        overflow: document.documentElement.scrollWidth - document.documentElement.clientWidth,
      };
    });
    assert.ok(Math.abs(bounds.right - width) <= 1, `${width}px: no outer gutter after the scrollbar.`);
    assert.ok(bounds.articleWidth <= bounds.articleLimit + 1, 'Keep the existing reading measure.');
    assert.ok(Math.abs(bounds.sidebarLeft - Math.max(0, (width - 1600) / 2)) <= 1,
      'Preserve the centered sidebar alignment.');
    assert.ok(bounds.overflow <= 1);
    await page.close();
  }
});

test('a short desktop TOC stays sticky while the article scrolls', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({ javaScriptEnabled: false, viewport: { width: 1440, height: 900 } });
  await page.goto(`${server.origin}/webui/start/`);
  const main = page.locator('.main-content');
  const rail = page.locator('.toc-rail');
  await expect(rail).toHaveCSS('position', 'sticky');
  await expect(rail).toHaveCSS('overflow-y', 'visible');
  const before = await rail.boundingBox();
  const articleBefore = await page.locator('.doc-content').boundingBox();
  assert.ok(before);
  assert.ok(articleBefore);
  await main.evaluate(element => { element.scrollTop = 300; });
  await expect.poll(() => main.evaluate(element => element.scrollTop)).toBe(300);
  const after = await rail.boundingBox();
  const articleAfter = await page.locator('.doc-content').boundingBox();
  assert.ok(after);
  assert.ok(articleAfter);
  assert.ok(Math.abs(before.y - after.y) < 1, 'Keep the outline available while reading.');
  assert.ok(Math.abs(articleBefore.y - articleAfter.y - 300) < 1, 'Only the article should move.');
  const dimensions = await rail.evaluate(element => ({
    client: element.clientHeight, scroll: element.scrollHeight, offset: element.scrollTop,
  }));
  assert.ok(dimensions.scroll <= dimensions.client + 1);
  assert.equal(dimensions.offset, 0);
  const mainBox = await main.boundingBox();
  assert.ok(mainBox);
  await page.mouse.move(after.x + 40, mainBox.y + 100);
  await page.mouse.wheel(0, 360);
  await expect.poll(() => main.evaluate(element => element.scrollTop)).toBeGreaterThan(300);
  assert.equal(await rail.evaluate(element => element.scrollTop), 0);
});

test('native CSS follows the visible heading without JavaScript or changing the fragment', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  for (const query of ['', '?toc-test=1']) {
    const page = await browser.newPage({ javaScriptEnabled: false, viewport: { width: 1440, height: 900 } });
    await page.goto(server.origin + article + query + '#component-files');
    assert.equal(await page.evaluate(() => CSS.supports('scroll-target-group: auto')), true,
      'This regression exercises a browser with native scroll-target-group support.');
    const current = page.locator('.outline-desktop a:target-current');
    await expect(page.locator('.outline-desktop ol')).toHaveCSS('scroll-target-group', 'auto');
    const wheelTarget = await page.evaluate(() => {
      const heading = document.getElementById('attr-html-attributes');
      const main = document.querySelector('.main-content');
      if (!heading || !main) throw new Error('Missing wheel-scroll target');
      const bounds = main.getBoundingClientRect();
      return { x: bounds.x + 100, y: bounds.y + 100, delta: heading.getBoundingClientRect().top - bounds.top };
    });
    await page.mouse.move(wheelTarget.x, wheelTarget.y);
    await page.mouse.wheel(0, wheelTarget.delta);
    await expect(current).toHaveAttribute('href', '/webui/learn/interactivity/#attr-html-attributes');
    for (const id of ['component-files', 'attr-html-attributes', 'direct-method-calls', 'when-not-to-hydrate']) {
      await page.evaluate(target => {
        const heading = document.getElementById(target);
        if (!heading) throw new Error(`Missing heading: ${target}`);
        heading.scrollIntoView({ block: 'start', behavior: 'instant' });
      }, id);
      await expect(current).toHaveCount(1);
      await expect(current).toHaveAttribute('href', `/webui/learn/interactivity/#${id}`);
      await expect(current).toHaveCSS('text-decoration-line', 'underline');
      await expect(current).toHaveCSS('text-decoration-thickness', '2px');
      await expect(current).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
      assert.equal(new URL(page.url()).hash, '#component-files', 'Selection must follow scroll, not only :target.');
    }
    await expect(page.locator('.main-content')).toHaveCSS('scroll-snap-type', 'none');
    await page.close();
  }
});

test('a long sticky outline reveals the native current link without scrolling the article', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, reducedMotion: 'reduce' });
  await page.route('https://**', route => route.abort());
  await page.goto(server.origin + article);
  const main = page.locator('.main-content');
  const outline = page.locator('.outline-desktop ol');
  const current = outline.locator('a:target-current');
  const anchors = await outline.locator('a').evaluateAll(links => links.map(link => {
    const href = link.getAttribute('href');
    if (!href) throw new Error('Missing outline link target');
    return new URL(href, document.baseURI).hash.slice(1);
  }));
  assert.ok(anchors.length > 30, 'Exercise uneven sections and a genuinely overflowing outline.');
  await expect(outline).toHaveCSS('scrollbar-width', 'none');
  await expect(outline).toHaveCSS('scroll-behavior', 'auto');
  for (const id of anchors) {
    await page.evaluate(target => {
      const heading = document.getElementById(target);
      if (!heading) throw new Error(`Missing heading: ${target}`);
      heading.scrollIntoView({ block: 'start', behavior: 'instant' });
    }, id);
    const articleScroll = await main.evaluate(element => element.scrollTop);
    await expect(current).toHaveAttribute('href', `${article}#${id}`);
    await expect(current).toHaveAttribute('aria-current', 'location');
    await expect(outline.locator('[aria-current]')).toHaveCount(1);
    await expect.poll(() => current.evaluate(element => {
      const viewport = element.closest('ol');
      if (!viewport) throw new Error('Missing outline viewport');
      const link = element.getBoundingClientRect();
      const bounds = viewport.getBoundingClientRect();
      return link.top >= bounds.top - 1 && link.bottom <= bounds.bottom + 1 &&
        link.top >= 72 && link.bottom <= innerHeight;
    })).toBe(true);
    assert.equal(await main.evaluate(element => element.scrollTop), articleScroll,
      'Following the outline must never move the article or reset reading progress.');
  }
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await expect(outline).toHaveCSS('scroll-behavior', 'smooth');
});

test('long sticky outlines retain native keyboard scrolling without JavaScript', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({ javaScriptEnabled: false, viewport: { width: 1440, height: 700 } });
  await page.goto(server.origin + article);
  const outline = page.locator('.outline-desktop ol');
  await expect(outline).toHaveCSS('overflow-y', 'auto');
  const last = outline.locator('a').last();
  await last.focus();
  await expect.poll(() => outline.evaluate(element => element.scrollTop)).toBeGreaterThan(0);
  await expect.poll(() => last.evaluate(element => {
    const bounds = element.getBoundingClientRect();
    return bounds.top >= 72 && bounds.bottom <= innerHeight;
  })).toBe(true);
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(/#when-not-to-hydrate$/);
});

test('the mobile outline expands into normal page flow and retains every heading link', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({ javaScriptEnabled: false, viewport: { width: 390, height: 844 } });
  await page.goto(server.origin + article);
  const disclosure = page.locator('.outline-mobile');
  await disclosure.locator('summary').click();
  const nav = disclosure.locator('nav');
  await expect(nav).toHaveCSS('overflow-y', 'visible');
  const size = await nav.evaluate(element => ({ client: element.clientHeight, scroll: element.scrollHeight }));
  assert.ok(size.scroll <= size.client + 1, 'Do not create a second scrolling viewport inside the disclosure.');
  assert.ok(await nav.locator('a').count() > 30);
  await nav.locator('a[href$="#when-not-to-hydrate"]').click();
  await expect(page).toHaveURL(/#when-not-to-hydrate$/);
  await expect(nav.locator('a:target-current')).toHaveAttribute('href', '/webui/learn/interactivity/#when-not-to-hydrate');
});

test('ordinary TOC links remain usable when native section tracking is unavailable', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  await page.route('https://**', route => route.abort());
  await page.goto(server.origin + article);
  await page.addStyleTag({ content: '.toc-rail ol { scroll-target-group: none !important; }' });
  await expect(page.locator('.outline-desktop a:target-current')).toHaveCount(0);
  await page.locator('.outline-desktop a[href$="#attr-html-attributes"]').click();
  await expect(page).toHaveURL(/#attr-html-attributes$/);
  await expect.poll(() => page.locator('.main-content').evaluate(element => element.scrollTop)).toBeGreaterThan(1000);
  await expect(page.locator('.outline-desktop a:target-current')).toHaveCount(0);
  assert.equal(await page.locator('.toc-rail').evaluate(element => element.scrollTop), 0);
});

test('the full framework title remains readable without colliding with navigation controls', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  for (const width of [1440, 901, 900, 820, 769, 768, 521, 520, 390, 320]) {
    const page = await browser.newPage({ viewport: { width, height: 900 } });
    await page.route('https://**', route => route.abort());
    await page.goto(`${server.origin}/webui/`);
    await expect(page.locator('.logo span')).toHaveText('WebUI Framework');
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('The Modern Web, Without the Weight');
    const header = await page.locator('.nav-bar').boundingBox();
    const name = await page.locator('.logo span').boundingBox();
    const controls = await page.locator('docs-site-navigation').boundingBox();
    assert.ok(header && name && controls);
    assert.ok(name.y >= header.y && name.y + name.height <= header.y + header.height,
      `${width}px: show the full title inside the header.`);
    assert.ok(name.x + name.width <= controls.x + 1, `${width}px: the title must not overlap the navigation.`);
    const lines = await page.locator('.logo span').evaluate(element => {
      const range = document.createRange();
      range.selectNodeContents(element);
      return [...range.getClientRects()].map(rect => ({ left: rect.left, right: rect.right }));
    });
    assert.ok(lines.every(line => line.right <= controls.x - 8),
      `${width}px: actual text, not just its flex box, needs space beside the controls.`);
    if (width > 520) {
      const shortcutLines = await page.locator('docs-search kbd').evaluate(element => {
        const range = document.createRange();
        range.selectNodeContents(element);
        return [...range.getClientRects()].map(rect => rect.top);
      });
      assert.ok(Math.max(...shortcutLines) - Math.min(...shortcutLines) <= 2,
        `${width}px: keep the keyboard shortcut on one line.`);
    }
    const overflow = await page.locator('.nav-bar').evaluate(element => element.scrollWidth - element.clientWidth);
    assert.ok(overflow <= 1, `${width}px: the header must not overflow.`);
    await page.close();
  }
});
