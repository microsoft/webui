// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test, type Page } from '@playwright/test';
import {
  buildBrowserModule,
  compileFixture,
  createWorkspace,
  expectOriginalElements,
  hydrate,
  loadSsr,
  removeWorkspace,
} from './fixture.js';
import { directiveCases, type DirectiveCase } from './sources.js';

let workspace: string;
let browserModule: string;

test.beforeAll(async () => {
  workspace = createWorkspace();
  browserModule = await buildBrowserModule(workspace);
});

test.afterAll(() => {
  if (workspace) removeWorkspace(workspace);
});

for (const fixture of directiveCases) {
  test(`${fixture.name}: published metadata`, async () => {
    const { meta } = compileFixture(fixture, workspace);
    const sections = [meta, ...(meta.b ?? [])];
    for (const section of sections) {
      // Escaped native text such as &lt;if&gt; must not count as a directive.
      expect(section.h).not.toMatch(/<\/?(?:if|for)(?=[\s>])/i);
    }
    expect(sections.flatMap(section => section.c ?? [])).toHaveLength(fixture.conditions);
    expect(sections.flatMap(section => section.r ?? [])).toHaveLength(fixture.repeats);
    expect(meta.th).toBeUndefined();

    const betweenTags = fixture.name.startsWith('CRLF between');
    const slot = [1, betweenTags ? 1 : 0];
    const statePath = (property: string) => fixture.message ? `message.${property}` : property;
    if (fixture.kind === 'if') {
      expect(meta.h).toBe(betweenTags ? '<article>\r\n\r\n</article>' : '<article></article>');
      expect(meta.c).toEqual([[[0, [statePath('enabled')]], 0, slot]]);
      if (fixture.conditions === 2) {
        expect(meta.b?.[0].h).toBe('<footer></footer>');
        expect(meta.b?.[0].c).toEqual([[[1, [statePath('ready')]], 1, [1, 0]]]);
      }
    } else if (fixture.kind === 'for') {
      expect(meta.r).toEqual([['items', 'item', 0, slot]]);
      expect(meta.b?.[0].ag).toEqual([[1, 0, 1]]);
      expect(meta.b?.[0].eg).toEqual([['click', [['select', [['p', 'item.id']], 1]]]]);
    } else {
      const repeats = sections.flatMap(section => section.r ?? []);
      expect(repeats.map(repeat => [repeat[0], repeat[1], repeat[4]]))
        .toEqual([['items', 'item', 'id'], ['item.children', 'child', 'id']]);
      expect(meta.c?.map(condition => condition[0][1])).toEqual([['enabled'], ['ready']]);
      expect(meta.h).toContain('&lt;if');
      expect(meta.h).toContain('&lt;for');
      expect(sections.flatMap(section => section.a ?? []).map(attr => attr[0]))
        .not.toContain('key');
    }
  });

  test(`${fixture.name}: SSR hydration and updates`, async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', error => errors.push(error.message));
    const { html } = compileFixture(fixture, workspace);
    await loadSsr(page, html, browserModule);
    if (fixture.kind === 'keyed') {
      await exerciseKeyed(page);
    } else {
      await exerciseMinimal(page, fixture);
    }
    expect(errors).toEqual([]);
  });
}

async function exerciseMinimal(page: Page, fixture: DirectiveCase): Promise<void> {
  const buttons = page.locator('test-directive-whitespace button');
  const labels = fixture.kind === 'if' ? ['Action'] : ['Alpha', 'Beta'];
  await expect(buttons).toHaveText(labels);
  expect(await buttons.evaluateAll(elements =>
    elements.map(element => element.parentElement?.tagName)))
    .toEqual(labels.map(() => fixture.parent));

  await hydrate(page);
  await expectOriginalElements(page);
  await page.evaluate(() => {
    document.querySelector('test-directive-whitespace')!.replaceUnchanged();
  });
  await expect(buttons).toHaveCount(labels.length);
  await expectOriginalElements(page);
  await expect(page.locator('test-directive-whitespace if, test-directive-whitespace for')).toHaveCount(0);

  await page.evaluate(() => {
    const host = document.querySelector('test-directive-whitespace')!;
    host.label = 'Updated';
    host.message = { ...host.message, label: 'Updated' };
    host.items = host.items.map(item => ({ ...item, label: `${item.label}!` }));
    host.$flushUpdates();
  });
  await expect(buttons).toHaveText(fixture.kind === 'if' ? ['Updated'] : ['Alpha!', 'Beta!']);
  if (fixture.kind === 'for') {
    await buttons.nth(1).click();
    expect(await page.evaluate(() =>
      document.querySelector('test-directive-whitespace')!.selected)).toBe('b');
  }

  await page.evaluate(({ kind, message }) => {
    const host = document.querySelector('test-directive-whitespace')!;
    if (message) host.message = { ...host.message, enabled: false };
    else if (kind === 'if') host.enabled = false;
    else host.items = [];
    host.$flushUpdates();
  }, fixture);
  await expect(buttons).toHaveCount(0);
  expect(await page.evaluate(() => window.__directiveWhitespaceSsr
    .filter(element => element.tagName === 'BUTTON')
    .every(element => !element.isConnected))).toBe(true);

  await page.evaluate(({ kind, message }) => {
    const host = document.querySelector('test-directive-whitespace')!;
    if (message) host.message = { ...host.message, enabled: true };
    else if (kind === 'if') host.enabled = true;
    else host.items = [{ id: 'c', label: 'New', visible: true, detail: true, children: [] }];
    host.$flushUpdates();
  }, fixture);
  await expect(buttons).toHaveText(fixture.kind === 'if' ? ['Updated'] : ['New']);
  expect(await buttons.evaluateAll(elements =>
    elements.map(element => element.parentElement?.tagName))).toEqual([fixture.parent]);

  if (fixture.conditions === 2) {
    await page.evaluate(() => {
      const host = document.querySelector('test-directive-whitespace')!;
      host.ready = false;
      host.message = { ...host.message, ready: false };
      host.$flushUpdates();
    });
    await expect(buttons).toHaveCount(0);
    await expect(page.locator('test-directive-whitespace article > footer')).toHaveCount(1);
  }
}

async function exerciseKeyed(page: Page): Promise<void> {
  const root = page.locator('test-directive-whitespace');
  await expect(root.locator('button')).toHaveCount(7);
  await expect(root.locator('article > .rows > div > .pick')).toHaveText(['Alpha', 'Beta']);
  await expect(root.locator('article > .rows > div > .child'))
    .toHaveText(['A child 1', 'A child 2', 'B child 1', 'B child 2']);
  await expect(root.locator('article > footer > .tail')).toHaveText('Action');
  await hydrate(page);
  await expectOriginalElements(page);
  await page.evaluate(() => {
    document.querySelector('test-directive-whitespace')!.replaceUnchanged();
  });
  await expect(root.locator('button')).toHaveCount(7);
  await expectOriginalElements(page);
  await expect(root.locator('if, for, [key]')).toHaveCount(0);

  await page.evaluate(() => {
    const host = document.querySelector('test-directive-whitespace')!;
    host.items = [...host.items].reverse().map(item => ({
      ...item, label: `${item.label}!`, children: [...item.children].reverse().map(child => ({ ...child })),
    }));
    host.$flushUpdates();
  });
  await expect(root.locator('.pick')).toHaveText(['Beta!', 'Alpha!']);
  await expect(root.locator('.child')).toHaveText(['B child 2', 'B child 1', 'A child 2', 'A child 1']);
  expect(await root.locator('.pick').evaluateAll(elements =>
    elements.map(element => (element as HTMLButtonElement).value))).toEqual(['Beta!', 'Alpha!']);
  expect(await page.evaluate(() => {
    const root = document.querySelector('test-directive-whitespace')!.shadowRoot!;
    return window.__directiveWhitespaceSsr.every((element, index) => element.isConnected
      && element.parentNode === window.__directiveWhitespaceSsrParents[index])
      && Array.from(root.querySelectorAll('[data-id]')).every(element =>
        window.__directiveWhitespaceKeyedSsr.get(
          `${element.tagName}.${element.className}:${element.getAttribute('data-id')}`,
        ) === element);
  })).toBe(true);
  await expect(root.locator('button')).toHaveCount(7);
  await root.locator('.pick').first().click();
  await expect(root.locator('output')).toHaveText('b');
  await root.locator('.child').last().click();
  await expect(root.locator('output')).toHaveText('a1');
  await root.locator('.tail').click();
  await expect(root.locator('output')).toHaveText('tail');

  await page.evaluate(() => {
    const host = document.querySelector('test-directive-whitespace')!;
    host.items = host.items.map(item => ({ ...item, visible: item.id !== 'a', detail: true }));
    host.ready = false;
    host.$flushUpdates();
  });
  await expect(root.locator('button')).toHaveCount(3);
  await expect(root.locator('article > .rows > div > .pick')).toHaveText(['Beta!']);
  await expect(root.locator('.detail')).toHaveText(['Beta!']);
  await expect(root.locator('footer')).toHaveCount(0);

  await page.evaluate(() => {
    const host = document.querySelector('test-directive-whitespace')!;
    host.enabled = false;
    host.$flushUpdates();
  });
  await expect(root.locator('button, .rows, .detail')).toHaveCount(0);
  await expect(root.locator('output')).toHaveText('tail');
  await page.evaluate(() => {
    const host = document.querySelector('test-directive-whitespace')!;
    host.enabled = true;
    host.ready = true;
    host.items = host.items.map(item => ({ ...item, visible: true }));
    host.$flushUpdates();
  });
  await expect(root.locator('button')).toHaveCount(7);
  await expect(root.locator('article > .rows > div > .pick')).toHaveText(['Beta!', 'Alpha!']);
  await expect(root.locator('article > footer > .tail')).toHaveCount(1);
  await root.locator('.pick').last().click();
  await expect(root.locator('output')).toHaveText('a');
}
