// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { docsDist, startWebsite } from '../../../../packages/webui-press/tests/website-fixture.js';
import {
  BenchmarkInputError, buildBenchmarkView, METRIC_IDS, prepareBenchmarkEvidence,
} from '../benchmark-explorer/benchmark-data.js';
import { buildBenchmarkNotes, buildBenchmarkPreviews } from './benchmark-preview-data.js';

const sourceText = fs.readFileSync(path.join(
  docsDist, '..', '.webui-press', 'state', 'benchmark-summary.json',
), 'utf8');

function source(): unknown {
  return JSON.parse(sourceText);
}

test('homepage evidence defaults to RPS and retains full-view scales for four ordered metrics', () => {
  const evidence = prepareBenchmarkEvidence(source());
  const before = JSON.stringify(evidence);
  const previews = buildBenchmarkPreviews(evidence);
  assert.deepEqual(previews.map(preview => preview.id), [
    'noStreamingRequestsPerSecond', 'streamingRequestsPerSecond', 'jsHeapUsedBytes', 'rendererPrivateBytes',
  ]);
  assert.deepEqual(previews.map(preview => preview.label), ['RPS', 'Streaming', 'JS heap', 'Renderer memory']);
  assert.deepEqual(previews.map(preview => preview.selected), [true, false, false, false]);
  assert.deepEqual(previews.map(preview => preview.tabIndex), [0, -1, -1, -1]);
  assert.deepEqual(previews.map(preview => preview.maximumLabel), ['1,741.15', '1,569.3', '4.94', '57.01']);
  for (const preview of previews) {
    const view = buildBenchmarkView(evidence, preview.id);
    assert.deepEqual(preview.rows.map(row => row.caseId), ['webui-rust-actix', 'react', 'next']);
    assert.deepEqual(preview.rows.map(row => row.label), ['WebUI', 'React', 'Next.js']);
    assert.equal(view.rows.find(row => row.caseId === 'webui-rust-actix')?.label, 'WebUI / Light DOM / Rust / Actix');
    assert.equal(preview.unit, view.activeMetric.unit);
    assert.equal(preview.maximumLabel, view.maximumLabel);
    assert.equal(preview.midpointLabel, view.midpointLabel);
    assert.equal(preview.directionLabel, view.activeMetric.direction === 'higher' ? 'Higher is better' : 'Lower is better');
    for (const row of preview.rows) {
      const full = view.rows.find(candidate => candidate.caseId === row.caseId);
      assert.ok(full);
      assert.equal(row.barWidth, full.barWidth);
      assert.equal(row.valueLabel, full.valueLabel);
      assert.ok(!('values' in row), 'do not copy all metrics or telemetry into each projected row');
    }
  }
  assert.deepEqual(previews.map(preview => preview.rows[0].barWidth), [
    100, 100, 43.281342630766886, 74.3713600548133,
  ]);
  assert.equal(JSON.stringify(evidence), before);
  assert.equal(JSON.stringify(buildBenchmarkPreviews(evidence)), JSON.stringify(previews));
});

test('homepage evidence keeps the full scale when an omitted configuration has the largest value', () => {
  const evidence = prepareBenchmarkEvidence(source());
  const omitted = evidence.rows.find(row => row.caseId === 'webui-shadowdom-rust-actix');
  assert.ok(omitted);
  for (const metric of evidence.metrics) {
    const maximum = Math.max(...evidence.rows.map(row => row.values[metric.id].value ?? 0));
    omitted.values[metric.id].value = maximum * 2;
  }
  for (const preview of buildBenchmarkPreviews(evidence)) {
    assert.ok(preview.rows.every(row => row.caseId !== omitted.caseId));
    assert.ok(preview.rows.every(row => row.barWidth !== null && row.barWidth <= 50),
      'Do not rescale to the three featured configurations.');
    assert.equal(preview.maximumLabel, buildBenchmarkView(evidence, preview.id).maximumLabel);
  }
});

test('homepage evidence rejects missing configurations instead of substituting a better result', () => {
  for (const caseId of ['webui-rust-actix', 'react', 'next']) {
    const evidence = prepareBenchmarkEvidence(source());
    evidence.rows = evidence.rows.filter(row => row.caseId !== caseId);
    assert.throws(() => buildBenchmarkPreviews(evidence), error =>
      error instanceof BenchmarkInputError && error.code === 'benchmark-preview' && error.field === caseId);
  }
});

test('homepage evidence preserves unavailable and measured-zero values', () => {
  const evidence = prepareBenchmarkEvidence(source());
  const missing = evidence.rows.find(row => row.caseId === 'react');
  const zero = evidence.rows.find(row => row.caseId === 'next');
  assert.ok(missing && zero);
  for (const id of METRIC_IDS) {
    missing.values[id] = { value: null, display: 'Unavailable', detail: 'Not recorded.' };
    zero.values[id] = { value: 0, display: '0', detail: 'Synthetic measured zero.' };
  }
  for (const preview of buildBenchmarkPreviews(evidence)) {
    assert.equal(preview.rows[1].available, false);
    assert.equal(preview.rows[1].barWidth, null);
    assert.equal(preview.rows[1].valueLabel, 'Unavailable');
    assert.equal(preview.rows[2].zero, true);
    assert.equal(preview.rows[2].barWidth, 0);
    assert.equal(preview.rows[2].valueLabel, `0 ${preview.unit}`);
  }
});

test('benchmark notes use the chart metric order and preserve sampling and measurement limitations', () => {
  const evidence = prepareBenchmarkEvidence(source());
  const previews = buildBenchmarkPreviews(evidence);
  const notes = buildBenchmarkNotes(evidence);
  assert.deepEqual(notes.map(note => note.id), previews.map(preview => preview.id));
  assert.deepEqual(notes.map(note => note.label), previews.map(preview => preview.caption));
  assert.match(notes[0].samplingLabel, /1 measured round \(20 s\); variability is not established/);
  assert.equal(notes[1].samplingLabel, notes[0].samplingLabel);
  assert.match(notes[0].description, /64 connections; response sizes differ/);
  assert.match(notes[1].description, /buffered routes.*resumable async data-boundary workload was not measured/);
  assert.match(notes[2].description, /garbage collection, not total browser memory/);
  assert.match(notes[3].description, /isolated Chromium renderer.*not server RSS/);
  assert.match(notes[3].description, /samples were unavailable.*sample counts/);
  assert.equal(notes[2].samplingLabel, '10 isolated Chromium desktop runs.');
  assert.equal(notes[3].samplingLabel, notes[2].samplingLabel);
  evidence.metrics = evidence.metrics.filter(metric => metric.id !== notes[0].id);
  assert.throws(() => buildBenchmarkNotes(evidence), error =>
    error instanceof BenchmarkInputError && error.code === 'benchmark-preview' && error.field === notes[0].id);
});

const metrics = [
  {
    label: 'RPS', id: 'noStreamingRequestsPerSecond', maximum: '1,741.15',
    values: ['1,741.15 req/s', '490.55 req/s', '151.2 req/s'],
    widths: [100, 28.173908049277774, 8.683915802774028],
    caption: 'Complete responses (req/s)',
    sampling: '1 measured round (20 s)', direction: 'Higher is better',
  },
  {
    label: 'Streaming', id: 'streamingRequestsPerSecond', maximum: '1,569.3',
    values: ['1,569.3 req/s', '286.3 req/s', '154.4 req/s'],
    widths: [100, 18.243802969476835, 9.838781622379402],
    caption: 'Streaming responses (req/s)',
    sampling: '1 measured round (20 s)', direction: 'Higher is better',
  },
  {
    label: 'JS heap', id: 'jsHeapUsedBytes', maximum: '4.94',
    values: ['2.14 MiB', '2.96 MiB', '3.75 MiB'],
    widths: [43.281342630766886, 60.01838373734428, 75.95260394439306],
    caption: 'JavaScript heap after GC (MiB)',
    sampling: '10 isolated Chromium desktop runs', direction: 'Lower is better',
  },
  {
    label: 'Renderer memory', id: 'rendererPrivateBytes', maximum: '57.01',
    values: ['42.4 MiB', '48.19 MiB', '48.18 MiB'],
    widths: [74.3713600548133, 84.51867077766359, 84.50154162384378],
    caption: 'Renderer private bytes (MiB)',
    sampling: '10 isolated Chromium desktop runs', direction: 'Lower is better',
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
    await expect(preview.locator('.chart-caption')).toContainText(metric.caption);
    await expect(preview.locator('.direction')).toHaveText(metric.direction);
    await expect(preview.locator('.chart-caption sup a')).toHaveAttribute(
      'href', `/webui/reference/benchmarks/#benchmark-note-${metric.id}`);
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
    await expect(preview.locator('.details-link')).toHaveAttribute('href', '/webui/reference/benchmarks/');
    await expect(preview.getByRole('link', { name: 'More frameworks and methodology', exact: true })).toBeVisible();
  }
  assert.deepEqual(errors, []);
  assert.equal(await page.evaluate(() => customElements.get('benchmark-explorer') !== undefined), false);
});

test('metric notes link to the benchmark page with and without JavaScript', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  for (const javaScriptEnabled of [false, true]) {
    const page = await browser.newPage({ javaScriptEnabled });
    await page.route('https://**', route => route.abort());
    const preview = page.locator('benchmark-preview');
    for (const metric of javaScriptEnabled ? metrics : metrics.slice(0, 1)) {
      await page.goto(`${server.origin}/webui/`);
      await expect(page.locator('.benchmark-notes')).toHaveCount(0);
      if (javaScriptEnabled) {
        const tab = preview.getByRole('tab', { name: metric.label, exact: true });
        await expect(tab).toBeEnabled();
        await tab.click();
      }
      const link = preview.locator('.chart-caption sup a');
      await expect(link).toHaveAccessibleName(/measurement notes/);
      await link.click();
      await expect(page).toHaveURL(`${server.origin}/webui/reference/benchmarks/#benchmark-note-${metric.id}`);
      const note = page.locator(`#benchmark-note-${metric.id}`);
      assert.equal(await note.evaluate(element => element.getRootNode() === document), true);
      await expect(note).toBeInViewport();
      await expect(note).toBeFocused();
      await expect(note).toContainText(metric.sampling);
    }
    await page.close();
  }
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
  const layouts = [1280, 1100, 820, 481, 480, 390, 320].flatMap(width =>
    ['theme', 'fallback'].map(font => ({ width, font })));
  for (const { width, font } of layouts) {
    const page = await browser.newPage({ viewport: { width, height: 800 } });
    await page.route('https://**', route => route.abort());
    await page.goto(`${server.origin}/webui/`);
    if (font === 'fallback') {
      await page.addStyleTag({ content: ':root { --docs-font-mono: "Courier New", monospace; }' });
    }
    const preview = page.locator('benchmark-preview');
    for (const metric of metrics) {
      const tab = preview.getByRole('tab', { name: metric.label, exact: true });
      await expect(tab).toBeEnabled();
      await tab.click();
      const bounds = await preview.boundingBox();
      const plot = await preview.locator('.plot').first().boundingBox();
      const scale = await preview.locator('.scale').boundingBox();
      const caption = await preview.locator('.chart-caption').boundingBox();
      const table = await preview.locator('table').boundingBox();
      assert.ok(bounds && plot && scale && caption && table);
      assert.ok(Math.abs(caption.width - table.width) <= 1,
        `${width}px ${metric.label}: the metric caption must use the full chart width.`);
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
      assert.equal(labels.length, bounds.width <= 440 ? 2 : 3);
      assert.ok(Math.abs(labels[0].left - plot.x) <= 1);
      assert.ok(Math.abs(labels[labels.length - 1].right - plot.x - plot.width) <= 1,
        `${width}px ${metric.label}: the maximum label must end at the plotted maximum.`);
      if (labels.length === 3) {
        assert.ok(Math.abs((labels[1].left + labels[1].right) / 2 - plot.x - plot.width / 2) <= 1,
          `${width}px ${metric.label}: the midpoint label must center on the plotted midpoint.`);
      }
      for (let index = 1; index < labels.length; index++) {
        assert.ok(labels[index].left - labels[index - 1].right >= 4,
          `${width}px ${font} ${metric.label}: Axis labels need separation, not overlap.`);
      }
      const dimensions = await preview.evaluate(element => ({
        width: element.clientWidth,
        scroll: element.shadowRoot?.querySelector('.preview')?.scrollWidth,
      }));
      assert.ok(dimensions.scroll !== undefined && dimensions.scroll <= dimensions.width,
        `${width}px ${metric.label}: no clipped chart content.`);
      await expect(preview.locator('.value')).toHaveText(metric.values);
      const typography = await preview.evaluate(element => {
        const root = element.shadowRoot;
        if (!root) throw new Error('Missing benchmark preview');
        return [...root.querySelectorAll('.metric-tabs button, tbody th, .value, .direction, .details-link')]
          .map(label => ({ label: label.textContent, size: Number.parseFloat(getComputedStyle(label).fontSize) }));
      });
      for (const { label, size } of typography) {
        assert.ok(size >= 14, `${width}px ${metric.label}: "${label}" must be at least 14px.`);
      }
      const values = await preview.locator('.value').evaluateAll(elements => elements.map(element => {
        const range = document.createRange();
        range.selectNodeContents(element);
        const text = range.getBoundingClientRect();
        const bounds = element.getBoundingClientRect();
        return { width: bounds.width, textWidth: text.width, lines: range.getClientRects().length };
      }));
      assert.ok(values.every(value => value.lines === 1 && value.textWidth <= value.width + 1),
        `${width}px ${metric.label}: larger values and units must fit on one line.`);
    }
    await page.close();
  }
});
