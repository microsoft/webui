// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test, type Locator, type Page } from '@playwright/test';
import type { DetailsFixture, DetailsPanel } from './element.js';

type PanelState = Partial<Pick<DetailsPanel,
  'details' | 'nonemptyDetails' | 'first' | 'middle' | 'last' | 'showMiddle' | 'items'>>;

async function update(panel: Locator, state: PanelState): Promise<void> {
  await panel.evaluate(async (element, values) => {
    const component = element as DetailsPanel;
    Object.assign(component, values);
    await component.$whenRendered();
  }, state);
}

async function assigned(card: Locator): Promise<(string | null)[]> {
  return card.evaluate(element =>
    element.shadowRoot!.querySelector('slot')!.assignedNodes().map(node => node.textContent));
}

async function expectSlot(card: Locator, text: string): Promise<void> {
  expect(await assigned(card)).toEqual([text]);
  await expect(card.locator('.fallback')).toBeHidden();
}

async function expectAbsent(card: Locator): Promise<void> {
  expect(await assigned(card)).toEqual([]);
  await expect(card.locator('.fallback')).toBeVisible();
}

async function hydrate(page: Page, mode: 'hydrated' | 'client-created'): Promise<Locator> {
  await page.goto('/empty-text-slot/fixture.html');
  const panel = page.locator('#ssr');
  await expect(panel).toHaveJSProperty('$ready', true);
  await panel.evaluate(async element => { await (element as DetailsPanel).$whenRendered(); });
  if (mode === 'hydrated') return panel;
  await page.locator('details-fixture').evaluate(async element => {
    const fixture = element as DetailsFixture;
    fixture.showClient = true;
    await fixture.$whenRendered();
  });
  const client = page.locator('#client');
  await expect(client).toHaveJSProperty('$ready', true);
  return client;
}

for (const initial of [
  { card: 'empty', property: 'details', text: '' },
  { card: 'nonempty', property: 'nonemptyDetails', text: 'Server details' },
] as const) {
  test(`${initial.card} SSR survives hydration and clearing without replacing adopted text`, async ({ page }) => {
    let release!: () => void;
    const allowed = new Promise<void>(resolve => { release = resolve; });
    await page.route('**/dist/empty-text-slot/element.js', async route => {
      await allowed;
      await route.continue();
    });
    await page.goto('/empty-text-slot/fixture.html', { waitUntil: 'commit' });
    const panel = page.locator('#ssr');
    const card = panel.locator(`#${initial.card}`);
    await expect(card.locator('slot')).toHaveCount(1);
    if (initial.text === '') await expectAbsent(card);
    else await expectSlot(card, initial.text);
    await card.evaluate(element => {
      Reflect.set(window, '__adoptedText', Array.from(element.childNodes).find(node => node.nodeType === 3));
    });
    release();
    await expect(panel).toHaveJSProperty('$ready', true);
    await panel.evaluate(async element => { await (element as DetailsPanel).$whenRendered(); });
    await expectSlot(card, initial.text);
    await update(panel, { [initial.property]: '' });
    await expectSlot(card, '');
    await update(panel, { [initial.property]: 'Updated' });
    await expectSlot(card, 'Updated');
    if (initial.text) {
      expect(await card.evaluate(element => element.shadowRoot!.querySelector('slot')!.assignedNodes()[0]
        === Reflect.get(window, '__adoptedText'))).toBe(true);
    }
    await update(panel, { [initial.property]: '' });
    await expectSlot(card, '');
  });
}

for (const mode of ['hydrated', 'client-created'] as const) {
  test(`${mode} preserves falsy values, whitespace, mixed runs and ordinary text`, async ({ page }) => {
    const panel = await hydrate(page, mode);
    for (const [value, text] of [
      [null, ''], [undefined, ''], [0, '0'], [false, 'false'], [' ', ' '],
      ['Details', 'Details'], ['', ''],
    ] as const) {
      await update(panel, { details: value });
      await expectSlot(panel.locator('#empty'), text);
      await expectSlot(panel.locator('#mixed'), `Before ${text} after`);
      await expectSlot(panel.locator('#whitespace'), ` ${text} `);
      await expect(panel.locator('#ordinary')).toHaveText(text);
    }
  });

  test(`${mode} orders native text around an initially empty active conditional`, async ({ page }) => {
    const panel = await hydrate(page, mode);
    const card = panel.locator('#adjacent');
    await update(panel, { middle: 'B' });
    expect(await assigned(card)).toEqual(['', 'B', '']);
    await update(panel, { last: 'C' });
    await update(panel, { first: 'A' });
    expect(await assigned(card)).toEqual(['A', 'B', 'C']);
    await update(panel, { first: '', middle: '', last: '' });
    expect(await assigned(card)).toEqual(['', '', '']);
    await update(panel, { showMiddle: false, middle: 'Removed' });
    await update(panel, { first: 'Left', last: 'Right' });
    expect(await assigned(card)).toEqual(['Left', 'Right']);
    await update(panel, { showMiddle: true, middle: 'Middle' });
    expect(await assigned(card)).toEqual(['Left', 'Middle', 'Right']);
  });

  test(`${mode} nested projected repeats keep text identity and release removed items`, async ({ page }) => {
    const panel = await hydrate(page, mode);
    await update(panel, { first: 'Left', last: 'Right' });
    for (const count of [3, 1000, 0, 7]) {
      const items = Array.from({ length: count }, (_, id) => ({ id, value: id % 2 ? `Item ${id}` : '' }));
      await update(panel, { items });
      const values = items.map(item => item.value);
      expect(await assigned(panel.locator('#repeat'))).toEqual(values);
      expect(await assigned(panel.locator('#nested'))).toEqual(['Left', ...values, 'Right']);
      expect(await assigned(panel.locator('#item-conditions'))).toEqual(values);
      await update(panel, { items: items.map(item => ({ ...item, value: '' })) });
      expect(await assigned(panel.locator('#repeat'))).toEqual(Array(count).fill(''));
      expect(await assigned(panel.locator('#nested'))).toEqual(['Left', ...Array(count).fill(''), 'Right']);
      await update(panel, { showMiddle: false });
      await expectAbsent(panel.locator('#item-conditions'));
      expect(await assigned(panel.locator('#nested'))).toEqual(['Left', 'Right']);
      await update(panel, { items: items.slice().reverse(), showMiddle: true });
      expect(await assigned(panel.locator('#nested'))).toEqual(['Left', ...values.slice().reverse(), 'Right']);
    }
    await update(panel, { items: [], first: '', last: '' });
    expect(await assigned(panel.locator('#nested'))).toEqual(['', '']);
    await expectAbsent(panel.locator('#repeat'));
    await update(panel, { showMiddle: false });
    await update(panel, { showMiddle: true });
    expect(await assigned(panel.locator('#nested'))).toEqual(['', '']);
  });

  test(`${mode} filled repeats retain no displaced empty positions`, async ({ page }) => {
    const panel = await hydrate(page, mode);
    await update(panel, { showMiddle: false, items: Array.from({ length: 40 }, () => ({ value: '' })) });
    await update(panel, { showMiddle: true });
    expect(await panel.locator('#nested').evaluate(element => ({
      text: Array.from(element.childNodes).filter(node => node.nodeType === 3).length,
      positions: Array.from(element.childNodes).filter(node => node.nodeType === 8).length,
    }))).toEqual({ text: 42, positions: 1 });
    await update(panel, { items: Array.from({ length: 40 }, () => ({ value: 'Text' })) });
    const retained = await panel.evaluate(element => {
      const root = (element as DetailsPanel)['$root'];
      if (!root) throw new Error('Missing hydrated root');
      const pending = [root];
      let detachedPositions = 0;
      while (pending.length) {
        const instance = pending.pop()!;
        for (const node of instance.nodes) {
          if (node.nodeType === 8 && !node.parentNode) detachedPositions++;
        }
        for (const condition of instance.conds) {
          if (condition.instance) pending.push(condition.instance);
        }
        for (const repeat of instance.repeats) pending.push(...repeat.instances);
      }
      return detachedPositions;
    });
    expect(retained).toBe(0);
    expect(await assigned(panel.locator('#nested'))).toEqual(['', ...Array(40).fill('Text'), '']);
  });
}
