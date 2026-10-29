// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { chromium, expect } from '@playwright/test';
import path from 'node:path';
import test from 'node:test';
import { build, fixture, removeFixtureRoot, write } from './native-fixture.js';
import { startWebsite } from './website-fixture.js';

function tabs(id: string, sync: string, values: string[], active = 'npm', stored = false): string {
  return `<webui-press-tabs id="${id}"${sync ? ` sync="${sync}"` : ''}${stored ? ' storage="local"' : ''}>
${values.map(value =>
    `<webui-press-tab slot="tab" value="${value}"${value === active ? ' active' : ''}>${value}</webui-press-tab>`).join('\n')}
${values.map(value =>
    `<webui-press-tab-panel${value === active ? ' active' : ''}><p>${id}:${value}</p></webui-press-tab-panel>`).join('\n')}
</webui-press-tabs>`;
}

test('synchronized tabs share page-local values across order, subsets and reconnects', async t => {
  const f = fixture();
  t.after(() => removeFixtureRoot(f.root));
  write(f.configFile, JSON.stringify({ ...f.config, regions: {} }));
  write(path.join(f.site, 'content/index.md'), '# Tab control fixtures\n');
  write(path.join(f.site, 'content/tabs.md'), `# Tab selection

${tabs('first', 'package-manager', ['npm', 'pnpm', 'yarn', 'bun'])}
${tabs('reordered', 'package-manager', ['bun', 'yarn', 'pnpm', 'npm'])}
${tabs('subset', 'package-manager', ['npm', 'bun'])}
${tabs('other', 'another-group', ['npm', 'pnpm'])}
${tabs('independent', '', ['npm', 'pnpm'], 'pnpm')}
${tabs('outer', '', ['npm', 'pnpm']).replace(
    '<p>outer:npm</p>', tabs('nested', 'another-group', ['npm', 'pnpm']))}
`);
  build(f.site);
  const server = await startWebsite(path.join(f.site, 'dist'), '/fixture/');
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const noScript = await browser.newPage({ javaScriptEnabled: false });
  await noScript.goto(`${server.origin}/fixture/tabs/`);
  await expect(noScript.locator('#first webui-press-tab-panel[active]')).toHaveText('first:npm');
  await expect(noScript.locator('#independent webui-press-tab-panel[active]')).toHaveText('independent:pnpm');
  await noScript.close();

  const page = await browser.newPage();
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => {
    if (message.type() === 'error') errors.push(message.text());
  });
  await page.goto(`${server.origin}/fixture/tabs/`);
  const first = page.locator('#first');
  const reordered = page.locator('#reordered');
  const subset = page.locator('#subset');
  await first.getByRole('button', { name: 'pnpm', exact: true }).click();
  await expect(reordered.locator('webui-press-tab-panel[active]')).toHaveText('reordered:pnpm');
  await expect(subset.locator('webui-press-tab-panel[active]')).toHaveText('subset:npm');
  await expect(page.locator('#other webui-press-tab-panel[active]')).toHaveText('other:npm');
  await expect(page.locator('#independent webui-press-tab-panel[active]')).toHaveText('independent:pnpm');
  await reordered.getByRole('button', { name: 'bun', exact: true }).click();
  await expect(first.locator('webui-press-tab-panel[active]')).toHaveText('first:bun');
  await expect(subset.locator('webui-press-tab-panel[active]')).toHaveText('subset:bun');

  const detached = await reordered.elementHandle();
  if (!detached) throw new Error('Missing reordered tab group');
  await detached.evaluate(element => element.remove());
  await first.getByRole('button', { name: 'yarn', exact: true }).click();
  expect(await detached.evaluate(element =>
    element.querySelector('webui-press-tab-panel[active]')?.textContent)).toBe('reordered:bun');
  await detached.evaluate(element => document.body.append(element));
  await expect(reordered.locator('webui-press-tab-panel[active]')).toHaveText('reordered:yarn');
  await reordered.getByRole('button', { name: 'npm', exact: true }).click();
  await expect(first.locator('webui-press-tab-panel[active]')).toHaveText('first:npm');
  await page.locator('#independent').getByRole('button', { name: 'pnpm', exact: true }).click();
  await expect(first.locator('webui-press-tab-panel[active]')).toHaveText('first:npm');
  await page.locator('#nested').getByRole('button', { name: 'pnpm', exact: true }).click();
  await expect(page.locator('#other webui-press-tab-panel[active]')).toHaveText('other:pnpm');
  await expect(page.locator('#outer > webui-press-tab[active]')).toHaveText('npm');
  await first.getByRole('button', { name: 'pnpm', exact: true }).click();

  const secondPage = await browser.newPage();
  await secondPage.goto(`${server.origin}/fixture/tabs/`);
  await expect(secondPage.locator('#first webui-press-tab-panel[active]')).toHaveText('first:npm');
  await expect(first.locator('webui-press-tab-panel[active]')).toHaveText('first:pnpm');
  await expect(page.locator('#independent webui-press-tab-panel[active]')).toHaveText('independent:pnpm');
  expect(errors).toEqual([]);
});

test('local tab preferences survive navigation and reloads without affecting other categories', async t => {
  const f = fixture();
  t.after(() => removeFixtureRoot(f.root));
  write(f.configFile, JSON.stringify({ ...f.config, regions: {} }));
  write(path.join(f.site, 'content/index.md'), '# Tab control fixtures\n');
  const content = `# Stored tab selection

${tabs('first', 'package-manager', ['npm', 'pnpm'], 'npm', true)}
${tabs('peer', 'package-manager', ['pnpm', 'npm'], 'npm', true)}
${tabs('subset', 'package-manager', ['npm'], 'npm', true)}
${tabs('language', 'language', ['rust', 'python'], 'rust', true)}
${tabs('ephemeral', 'temporary', ['npm', 'pnpm'])}
${tabs('early', 'late-category', ['npm', 'pnpm'])}
${tabs('late', 'late-category', ['npm', 'pnpm'])}
`;
  for (const name of ['tabs', 'next']) {
    write(path.join(f.site, 'content', `${name}.md`), content);
  }
  build(f.site);
  const server = await startWebsite(path.join(f.site, 'dist'), '/fixture/');
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const context = await browser.newContext();
  const page = await context.newPage();
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(`${server.origin}/fixture/tabs/`);
  await page.locator('#first').getByRole('button', { name: 'pnpm', exact: true }).click();
  await page.locator('#language').getByRole('button', { name: 'python', exact: true }).click();
  expect(await page.evaluate(() => localStorage.getItem('webui-press:tabs:package-manager'))).toBe('pnpm');
  expect(await page.evaluate(() => localStorage.getItem('webui-press:tabs:language'))).toBe('python');
  await page.evaluate(() => localStorage.setItem('webui-press:tabs:temporary', 'pnpm'));
  await page.goto(`${server.origin}/fixture/next/`);
  await expect(page.locator('#first webui-press-tab-panel[active]')).toHaveText('first:pnpm');
  await expect(page.locator('#peer webui-press-tab-panel[active]')).toHaveText('peer:pnpm');
  await expect(page.locator('#subset webui-press-tab-panel[active]')).toHaveText('subset:npm');
  await expect(page.locator('#language webui-press-tab-panel[active]')).toHaveText('language:python');
  await expect(page.locator('#ephemeral webui-press-tab-panel[active]')).toHaveText('ephemeral:npm');

  const secondPage = await page.context().newPage();
  await secondPage.goto(`${server.origin}/fixture/tabs/`);
  await expect(secondPage.locator('#first webui-press-tab-panel[active]')).toHaveText('first:pnpm');
  await secondPage.locator('#first').getByRole('button', { name: 'npm', exact: true }).click();
  await expect(page.locator('#first webui-press-tab-panel[active]')).toHaveText('first:pnpm');
  await page.reload();
  await expect(page.locator('#first webui-press-tab-panel[active]')).toHaveText('first:npm');

  await page.evaluate(() => localStorage.setItem('webui-press:tabs:package-manager', 'removed-manager'));
  await page.reload();
  await expect(page.locator('#first webui-press-tab-panel[active]')).toHaveText('first:npm');
  await page.locator('#first').getByRole('button', { name: 'pnpm', exact: true }).click();
  expect(await page.evaluate(() => localStorage.getItem('webui-press:tabs:package-manager'))).toBe('pnpm');

  const late = await page.locator('#late').elementHandle();
  if (!late) throw new Error('Missing late storage group');
  await late.evaluate(element => {
    element.remove();
    element.setAttribute('storage', 'local');
  });
  await page.evaluate(() => localStorage.setItem('webui-press:tabs:late-category', 'npm'));
  await page.locator('#early').getByRole('button', { name: 'pnpm', exact: true }).click();
  await late.evaluate(element => document.body.append(element));
  await expect(page.locator('#early webui-press-tab-panel[active]')).toHaveText('early:pnpm');
  await expect(page.locator('#late webui-press-tab-panel[active]')).toHaveText('late:pnpm');
  expect(await page.evaluate(() => localStorage.getItem('webui-press:tabs:late-category'))).toBe('pnpm');
  expect(errors).toEqual([]);
});

test('tab storage access, read and quota failures retain page-local synchronization with a warning', async t => {
  const f = fixture();
  t.after(() => removeFixtureRoot(f.root));
  write(f.configFile, JSON.stringify({ ...f.config, regions: {} }));
  write(path.join(f.site, 'content/index.md'), `# Storage failures

${tabs('first', 'package-manager', ['npm', 'pnpm'], 'npm', true)}
${tabs('peer', 'package-manager', ['pnpm', 'npm'], 'npm', true)}
`);
  build(f.site, '--show=content');
  const server = await startWebsite(path.join(f.site, 'dist'), '/fixture/');
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  for (const failure of ['access', 'read', 'write'] as const) {
    const page = await browser.newPage();
    const warnings: string[] = [];
    const errors: string[] = [];
    page.on('console', message => {
      if (message.type() === 'warning' && message.text().startsWith('[WebUI Press]')) {
        warnings.push(message.text());
      }
    });
    page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript(mode => {
      if (mode === 'access') {
        Object.defineProperty(window, 'localStorage', {
          get() { throw new DOMException('Storage blocked', 'SecurityError'); },
        });
      } else if (mode === 'read') {
        const getItem = Storage.prototype.getItem;
        Storage.prototype.getItem = function (key: string) {
          if (key.startsWith('webui-press:tabs:')) throw new DOMException('Read blocked', 'SecurityError');
          return getItem.call(this, key);
        };
      } else {
        const setItem = Storage.prototype.setItem;
        Storage.prototype.setItem = function (key: string, value: string) {
          if (key.startsWith('webui-press:tabs:')) throw new DOMException('Storage full', 'QuotaExceededError');
          setItem.call(this, key, value);
        };
      }
    }, failure);
    await page.goto(`${server.origin}/fixture/`);
    await page.locator('#first').getByRole('button', { name: 'pnpm', exact: true }).click();
    await expect(page.locator('#peer webui-press-tab-panel[active]')).toHaveText('peer:pnpm');
    await page.locator('#peer').getByRole('button', { name: 'npm', exact: true }).click();
    await expect(page.locator('#first webui-press-tab-panel[active]')).toHaveText('first:npm');
    expect(warnings).toHaveLength(1);
    expect(warnings[0]).toContain(failure === 'write' ? 'Cannot save tab preferences' : 'Cannot restore tab preferences');
    expect(errors).toEqual([]);
    await page.close();
  }
});
