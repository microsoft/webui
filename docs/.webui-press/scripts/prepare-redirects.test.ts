// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fixture, removeFixtureRoot, write } from '../../../packages/webui-press/tests/native-fixture.js';
import { docsDist, startWebsite } from '../../../packages/webui-press/tests/website-fixture.js';

const press = path.join(docsDist, '..', '.webui-press');

test('redirect preparation validates destinations and removes only obsolete generated files', t => {
  const f = fixture();
  t.after(() => removeFixtureRoot(f.root));
  const local = path.join(f.site, '.webui-press');
  const script = path.join(local, 'scripts/prepare-redirects.mjs');
  const routes = path.join(local, 'redirects.json');
  write(script, fs.readFileSync(path.join(press, 'scripts/prepare-redirects.mjs'), 'utf8'));
  write(f.configFile, JSON.stringify({ site: { title: 'Test docs', url: 'https://example.com' }, basePath: '/nested/' }));
  write(path.join(f.site, 'start/index.md'), '# Start\n');
  write(path.join(f.site, 'learn/topic.md'), '# Topic\n');
  write(routes, JSON.stringify({ '/guide/': '/start/', '/guide/topic/': '/learn/topic/' }));
  const run = () => execFileSync(process.execPath, [script], { cwd: f.site, encoding: 'utf8' });
  assert.match(run(), /Prepared 2 documentation redirects/);
  const first = path.join(local, 'public/guide/index.html');
  const second = path.join(local, 'public/guide/topic/index.html');
  const html = fs.readFileSync(first, 'utf8');
  assert.ok(html.includes('href="https://example.com/nested/start/"'));
  assert.ok(html.includes('href="/nested/start/"'));
  run();
  assert.equal(fs.readFileSync(first, 'utf8'), html);
  const unrelated = path.join(local, 'public/guide/keep.txt');
  write(unrelated, 'keep');
  write(routes, JSON.stringify({ '/guide/': '/start/' }));
  run();
  assert.equal(fs.existsSync(second), false);
  assert.equal(fs.readFileSync(unrelated, 'utf8'), 'keep');
  for (const invalid of [
    { '/../escape/': '/start/' },
    { '/guide/': '/learn/missing/' },
    { '/start/': '/learn/topic/' },
  ]) {
    write(routes, JSON.stringify(invalid));
    const failure = spawnSync(process.execPath, [script], { cwd: f.site, encoding: 'utf8' });
    assert.equal(failure.status, 1);
    assert.match(failure.stderr, /\[docs-redirects\].*help:/s);
    assert.equal(fs.readFileSync(first, 'utf8'), html);
  }
});

test('published legacy routes point to canonical pages with a no-script fallback', async t => {
  const routes: Record<string, string> = JSON.parse(fs.readFileSync(path.join(press, 'redirects.json'), 'utf8'));
  for (const [from, to] of Object.entries(routes)) {
    const html = fs.readFileSync(path.join(docsDist, from.slice(1), 'index.html'), 'utf8');
    assert.ok(html.includes(`href="https://microsoft.github.io/webui${to}"`));
    assert.ok(html.includes(`href="/webui${to}"`));
  }
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const noScript = await browser.newPage({ javaScriptEnabled: false });
  await noScript.goto(`${server.origin}/webui/guide/`);
  await expect(noScript).toHaveURL(`${server.origin}/webui/start/`);
  await expect(noScript.getByRole('heading', { level: 1 })).toContainText('What is WebUI?');
  await noScript.close();

  const page = await browser.newPage();
  await page.route('https://**', route => route.abort());
  await page.goto(`${server.origin}/webui/tutorials/hello-world/?from=bookmark#creating-the-template`);
  await expect(page).toHaveURL(`${server.origin}/webui/start/first-page/?from=bookmark#creating-the-template`);
  await expect(page.locator('#creating-the-template')).toBeInViewport();
});
