// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import test from 'node:test';
import { startWebsite } from '../../../packages/webui-press/tests/website-fixture.js';

const steps = [
  ['start', 'What is WebUI?'],
  ['start/installation', 'Installation'],
  ['start/first-page', 'Your first page'],
  ['start/first-component', 'Add a component'],
  ['start/add-interactivity', 'Add interaction'],
  ['start/todo-app', 'Build a Todo app'],
] as const;

test('primary navigation separates AI and keeps examples and benchmarks in Reference', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({ javaScriptEnabled: false, viewport: { width: 1280, height: 900 } });
  for (const [route, active] of [
    ['start/', 'Docs'], ['learn/', 'Docs'], ['build/', 'Docs'], ['reference/', 'Docs'],
    ['reference/examples/', 'Docs'], ['reference/benchmarks/', 'Docs'], ['ai/', 'AI'],
    ['playground/', 'Playground'],
  ]) {
    await page.goto(`${server.origin}/webui/${route}`);
    const primary = page.locator('docs-site-navigation .nav-links');
    await expect(primary.getByRole('link')).toHaveText(['Docs', 'AI', 'Playground', 'GitHub']);
    await expect(primary.locator('a[active]')).toHaveText(active);
  }
  await page.goto(`${server.origin}/webui/start/`);
  const reference = page.locator('.sidebar .sidebar-group').filter({
    has: page.getByText('Reference', { exact: true }),
  });
  await expect(reference.getByRole('link', { name: 'Examples', exact: true }))
    .toHaveAttribute('href', '/webui/reference/examples');
  await expect(reference.getByRole('link', { name: 'Benchmarks', exact: true }))
    .toHaveAttribute('href', '/webui/reference/benchmarks');
});

test('getting-started pages share Docs navigation and ordered lesson links without JavaScript', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({ javaScriptEnabled: false, viewport: { width: 1280, height: 900 } });
  let sidebarLinks: string[] | undefined;
  for (const [index, [route, label]] of steps.entries()) {
    await page.goto(`${server.origin}/webui/${route}/`);
    const sidebar = page.locator('.sidebar docs-sidebar-navigation');
    const links = await sidebar.locator('a').evaluateAll(elements =>
      elements.map(element => element.getAttribute('href') ?? ''));
    if (sidebarLinks) assert.deepEqual(links, sidebarLinks, `${route}: keep the documentation map.`);
    else sidebarLinks = links;
    await expect(sidebar.locator('.sidebar-link-active')).toHaveText(label);
    await expect(page.locator('docs-site-navigation .nav-links').getByRole('link', { name: 'Docs', exact: true }))
      .toHaveAttribute('active', '');
    const previous = steps[index - 1];
    const next = steps[index + 1];
    if (previous) {
      await expect(page.locator(`.page-nav a[href="/webui/${previous[0]}"]`)).toContainText(previous[1]);
    }
    if (next) {
      await expect(page.locator(`.page-nav a[href="/webui/${next[0]}"]`)).toContainText(next[1]);
    }
  }
});

test('mobile readers can move between a lesson and its reference without leaving Docs', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({ viewport: { width: 390, height: 844 } });
  await page.route('https://**', route => route.abort());
  await page.goto(`${server.origin}/webui/start/first-page/`);
  await page.getByRole('button', { name: 'Open navigation' }).click();
  const dialog = page.getByRole('dialog', { name: 'Browse WebUI' });
  await expect(dialog.locator('.mobile-primary-nav').getByRole('link'))
    .toHaveText(['Docs', 'AI', 'Playground', 'GitHub']);
  await expect(dialog.getByRole('link', { name: 'Docs', exact: true })).toHaveAttribute('active', '');
  await expect(dialog.locator('.sidebar-link-active')).toHaveText('Your first page');
  await dialog.getByRole('link', { name: 'Components', exact: true }).click();
  await expect(page).toHaveURL(`${server.origin}/webui/learn/components`);
  await page.waitForLoadState('load');
  await page.getByRole('button', { name: 'Open navigation' }).click();
  await expect(dialog.getByRole('link', { name: 'Docs', exact: true })).toHaveAttribute('active', '');
  await dialog.getByRole('link', { name: 'Your first page', exact: true }).click();
  await expect(page).toHaveURL(`${server.origin}/webui/start/first-page`);
});

test('mobile navigation covers the header, slides in, and respects reduced motion', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch();
  t.after(() => browser.close());
  const page = await browser.newPage({
    viewport: { width: 390, height: 844 }, reducedMotion: 'no-preference',
  });
  await page.goto(`${server.origin}/webui/reference/webui-press/`);
  const opener = page.getByRole('button', { name: 'Open navigation', exact: true });
  const dialog = page.locator('docs-site-navigation dialog.mobile-navigation');
  await dialog.evaluate(element => {
    for (const type of ['transitionrun', 'transitionend']) {
      element.addEventListener(type, event => {
        if (!(event instanceof TransitionEvent) || event.propertyName !== 'transform' || event.pseudoElement) return;
        const attribute = event.type === 'transitionrun' ? 'data-slide-start-x' : 'data-slide-end-x';
        element.setAttribute(attribute, String(element.getBoundingClientRect().x));
      });
    }
  });
  await opener.click();
  await expect(dialog).toBeVisible();
  await expect.poll(async () => (await dialog.boundingBox())?.y).toBe(0);
  await expect(dialog).toHaveCSS('height', '844px');
  await expect(dialog).toHaveAttribute('data-slide-end-x', /.+/);
  const slide = await dialog.evaluate(element => ({
    start: Number(element.getAttribute('data-slide-start-x')),
    end: Number(element.getAttribute('data-slide-end-x')),
  }));
  assert.ok(slide.start > slide.end, 'The drawer must move in from the right.');
  await dialog.getByRole('button', { name: 'Close navigation' }).click();
  await expect(dialog).toBeHidden();
  await expect(opener).toBeFocused();

  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.setViewportSize({ width: 320, height: 568 });
  await opener.click();
  await expect(dialog).toBeVisible();
  await expect.poll(async () => (await dialog.boundingBox())?.y).toBe(0);
  await expect(dialog).toHaveCSS('height', '568px');
  assert.equal(await dialog.evaluate(element => element.getAnimations().length), 0);
  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  await expect(opener).toBeFocused();
});
