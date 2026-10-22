// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import test from 'node:test';
import { startWebsite } from './website-fixture.js';

const metrics = [
  {
    label: 'RPS', id: 'noStreamingRequestsPerSecond', maximum: '1,593.4',
    values: ['1,593.4 req/s', '319.35 req/s', '138.4 req/s'],
    widths: [100, 20.042048449855653, 8.685829044809841],
    sampling: '1 measured round (20 s)', direction: 'Higher serves more requests',
  },
  {
    label: 'Streaming', id: 'streamingRequestsPerSecond', maximum: '1,388.45',
    values: ['1,388.45 req/s', '267.85 req/s', '141.6 req/s'],
    widths: [100, 19.291296049551658, 10.198422701573696],
    sampling: '1 measured round (20 s)', direction: 'Higher serves more requests',
  },
  {
    label: 'JS heap', id: 'jsHeapUsedBytes', maximum: '5.22',
    values: ['2.1 MiB', '2.92 MiB', '3.69 MiB'],
    widths: [40.24523190606928, 55.87096208890706, 70.7244143933641],
    sampling: '10 isolated Chromium desktop runs', direction: 'Lower uses less JS memory',
  },
  {
    label: 'Renderer memory', id: 'rendererPrivateBytes', maximum: '73.31',
    values: ['58.48 MiB', '63.72 MiB', '65.81 MiB'],
    widths: [79.77354469162115, 86.9188757159984, 89.76954842147329],
    sampling: '10 isolated Chromium desktop runs', direction: 'Lower uses less renderer memory',
  },
];

test('the homepage defaults to RPS and switches four ordered metrics without mounting hidden charts', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({ reducedMotion: 'reduce' });
  await page.route('https://**', route => route.abort());
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(`${server.origin}/webui/`);
  const preview = page.locator('benchmark-preview');
  const tabs = preview.getByRole('tab');
  await expect(tabs).toHaveCount(4);
  assert.deepEqual(await tabs.allTextContents(), metrics.map(metric => metric.label));
  await expect(preview.getByRole('tab', { name: 'RPS', exact: true })).toHaveAttribute('aria-selected', 'true');
  await expect(preview.getByRole('link')).toContainText('3 of 16 configurations');
  for (const metric of metrics) {
    const tab = preview.getByRole('tab', { name: metric.label, exact: true });
    await expect(tab).toBeEnabled();
    await tab.click();
    await expect(tab).toHaveAttribute('aria-selected', 'true');
    await expect(preview.getByRole('tab', { selected: true })).toHaveCount(1);
    await expect(preview.getByRole('tabpanel')).toHaveCount(1);
    await expect(preview.getByRole('tabpanel')).toHaveAttribute('id', `preview-panel-${metric.id}`);
    await expect(tab).toHaveAttribute('aria-controls', `preview-panel-${metric.id}`);
    await expect(preview.locator('table')).toHaveCount(1);
    await expect(preview.locator('.plot')).toHaveCount(3);
    const webui = preview.locator('tbody tr[data-webui]');
    await expect(webui).toHaveCount(1);
    await expect(webui).toHaveAttribute('data-case', 'webui-rust-actix');
    await expect(webui.locator('th')).toHaveText('WebUI');
    await expect(preview.locator('.value')).toHaveText(metric.values);
    await expect(preview.locator('.qualification')).toContainText(metric.sampling);
    await expect(preview.locator('.qualification')).toContainText('provenance are incomplete');
    await expect(preview.locator('.reading')).toContainText(metric.direction);
    await expect(preview.locator('.scale')).toContainText(metric.maximum);
    const widths = await preview.locator('.bar').evaluateAll(elements =>
      elements.map(element => Number(element.getAttribute('width'))));
    metric.widths.forEach((expected, index) => assert.ok(Math.abs(widths[index] - expected) < 1e-10));
    const shapes = await preview.locator('.bar').evaluateAll(elements => elements.map(element => ({
      graphics: element instanceof SVGGraphicsElement,
      namespace: element.namespaceURI,
      width: element.getBoundingClientRect().width,
      height: element.getBoundingClientRect().height,
    })));
    for (const shape of shapes) {
      assert.equal(shape.namespace, 'http://www.w3.org/2000/svg');
      assert.ok(shape.graphics && shape.width > 0 && shape.height > 0, 'Each selected bar must actually paint, not merely carry numeric attributes.');
    }
    await expect(preview.getByRole('link')).toHaveAttribute('href', '/webui/benchmarks/');
    await expect(preview.getByRole('link')).toContainText('3 of 16 configurations');
  }
  await expect(preview.locator('.context')).toContainText('private bytes, not RSS');
  assert.deepEqual(errors, []);
  assert.equal(await page.evaluate(() => customElements.get('benchmark-explorer') !== undefined), false);
});

test('benchmark bars display their full recorded values immediately without animations', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  for (const reducedMotion of ['no-preference', 'reduce'] as const) {
    const page = await browser.newPage({ reducedMotion });
    await page.route('https://**', route => route.abort());
    await page.goto(`${server.origin}/webui/`);
    const preview = page.locator('benchmark-preview');
    for (const metric of metrics) {
      const tab = preview.getByRole('tab', { name: metric.label, exact: true });
      await expect(tab).toBeEnabled();
      await tab.click();
      const appearances = await preview.locator('.bar').evaluateAll(elements => elements.map(element => {
        if (!(element instanceof SVGGraphicsElement) || !element.ownerSVGElement) {
          throw new Error('Expected a painted SVG bar');
        }
        const style = getComputedStyle(element);
        return {
          animation: style.animationName,
          delay: style.animationDelay,
          transition: style.transitionDuration,
          transform: style.transform,
          activeAnimations: element.getAnimations().length,
          width: element.getBoundingClientRect().width,
          expectedWidth: element.ownerSVGElement.getBoundingClientRect().width *
            Number(element.getAttribute('width')) / 100,
        };
      }));
      assert.equal(appearances.length, 3);
      for (const bar of appearances) {
        assert.equal(bar.animation, 'none');
        assert.equal(bar.delay, '0s');
        assert.equal(bar.transition, '0s');
        assert.equal(bar.transform, 'none');
        assert.equal(bar.activeAnimations, 0);
        assert.ok(bar.width > 0 && Math.abs(bar.width - bar.expectedWidth) < 1,
          'Show the full measured bar immediately, without waiting for an animation.');
      }
      await expect(preview.locator('.value')).toHaveText(metric.values);
    }
    await page.close();
  }
});

test('metric tabs support roving focus, arrow keys, Home and End', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage();
  await page.route('https://**', route => route.abort());
  await page.goto(`${server.origin}/webui/`);
  const preview = page.locator('benchmark-preview');
  const first = preview.getByRole('tab', { name: 'RPS', exact: true });
  await expect(first).toBeEnabled();
  await first.focus();
  for (const [key, label] of [
    ['ArrowRight', 'Streaming'], ['ArrowRight', 'JS heap'],
    ['End', 'Renderer memory'], ['ArrowRight', 'RPS'],
    ['ArrowLeft', 'Renderer memory'], ['Home', 'RPS'],
  ]) {
    await page.keyboard.press(key);
    const selected = preview.getByRole('tab', { name: label, exact: true });
    await expect(selected).toBeFocused();
    await expect(selected).toHaveAttribute('aria-selected', 'true');
    await expect(selected).toHaveAttribute('tabindex', '0');
    await expect(selected).toHaveCSS('outline-style', 'solid');
    await expect(preview.locator('[role="tab"][tabindex="0"]')).toHaveCount(1);
  }
  await page.keyboard.press('Tab');
  await expect(preview.getByRole('tabpanel')).toBeFocused();
});

test('all four previews retain readable values and aligned scales across viewport sizes', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  for (const width of [1280, 820, 390, 320]) {
    const page = await browser.newPage({ viewport: { width, height: 800 } });
    await page.route('https://**', route => route.abort());
    await page.goto(`${server.origin}/webui/`);
    const preview = page.locator('benchmark-preview');
    for (const metric of metrics) {
      const tab = preview.getByRole('tab', { name: metric.label, exact: true });
      await expect(tab).toBeEnabled();
      await tab.click();
      const bounds = await preview.boundingBox();
      const plot = await preview.locator('.plot').first().boundingBox();
      const scale = await preview.locator('.scale').boundingBox();
      assert.ok(bounds && plot && scale);
      if (width === 1280) assert.ok(bounds.y + bounds.height <= 800, `${metric.label}: keep the entire desktop preview visible.`);
      assert.ok(Math.abs(plot.x - scale.x) <= 1 && Math.abs(plot.width - scale.width) <= 1,
        `${width}px ${metric.label}: labels and plots must share a scale.`);
      const labels = await preview.locator('.scale span').evaluateAll(elements => elements
        .filter(element => getComputedStyle(element).visibility !== 'hidden')
        .map(element => {
          const range = document.createRange();
          range.selectNodeContents(element);
          const bounds = range.getBoundingClientRect();
          return { left: bounds.left, right: bounds.right };
        }));
      assert.ok(Math.abs(labels[0].left - plot.x) <= 1);
      assert.ok(Math.abs(labels[labels.length - 1].right - plot.x - plot.width) <= 1,
        `${width}px ${metric.label}: the maximum label must end at the plotted maximum.`);
      for (let index = 1; index < labels.length; index++) {
        assert.ok(labels[index].left - labels[index - 1].right >= 4, 'Axis labels need separation, not overlap.');
      }
      const dimensions = await preview.evaluate(element => ({
        width: element.clientWidth,
        scroll: element.shadowRoot?.querySelector('.preview')?.scrollWidth,
      }));
      assert.ok(dimensions.scroll !== undefined && dimensions.scroll <= dimensions.width,
        `${width}px ${metric.label}: no clipped chart content.`);
      await expect(preview.locator('.value')).toHaveText(metric.values);
    }
    await page.close();
  }
});
