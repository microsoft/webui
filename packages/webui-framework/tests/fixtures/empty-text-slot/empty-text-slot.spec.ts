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

async function expectSlot(card: Locator, text: string): Promise<void> {
  expect(await card.evaluate(element => {
    const slot = element.shadowRoot!.querySelector('slot')!;
    return slot.assignedNodes().map(node => ({
      type: node.nodeType,
      text: node.textContent,
    }));
  })).toEqual(text === '' ? [] : [{ type: 3, text }]);
  // A fallback remains in the shadow tree even when not rendered by the slot.
  if (text === '') await expect(card.locator('.fallback')).toBeVisible();
  else await expect(card.locator('.fallback')).toBeHidden();
}

async function hydrate(page: Page, mode: 'hydrated' | 'client-created' = 'hydrated'): Promise<Locator> {
  await page.goto('/empty-text-slot/fixture.html');
  const panel = page.locator('#ssr');
  await expect(panel).toHaveJSProperty('$ready', true);
  await panel.evaluate(async element => { await (element as DetailsPanel).$whenRendered(); });
  if (mode === 'client-created') {
    await page.locator('details-fixture').evaluate(async element => {
      const fixture = element as DetailsFixture;
      fixture.showClient = true;
      await fixture.$whenRendered();
    });
    const client = page.locator('#client');
    await expect(client).toHaveJSProperty('$ready', true);
    return client;
  }
  return panel;
}

for (const initial of [
  { card: 'empty', property: 'details', text: '' },
  { card: 'nonempty', property: 'nonemptyDetails', text: 'Server details' },
] as const) {
  test(`optional details: ${initial.card} SSR survives hydration and empty/text/empty updates`, async ({ page }) => {
    let release!: () => void;
    const hydrationAllowed = new Promise<void>(resolve => { release = resolve; });
    await page.route('**/dist/empty-text-slot/element.js', async route => {
      await hydrationAllowed;
      await route.continue();
    });
    await page.goto('/empty-text-slot/fixture.html', { waitUntil: 'commit' });
    const panel = page.locator('#ssr');
    const card = panel.locator(`#${initial.card}`);
    await expect(card.locator('slot')).toHaveCount(1);

    await expectSlot(card, initial.text);
    await card.evaluate(element => {
      Reflect.set(window, '__serverDetailsText',
        Array.from(element.childNodes).find(node => node.nodeType === Node.TEXT_NODE));
    });
    release();
    await expect(panel).toHaveJSProperty('$ready', true);
    await panel.evaluate(async element => { await (element as DetailsPanel).$whenRendered(); });
    await expectSlot(card, initial.text);
    if (initial.text) {
      expect(await card.evaluate(element =>
        element.shadowRoot!.querySelector('slot')!.assignedNodes()[0]
          === Reflect.get(window, '__serverDetailsText'))).toBe(true);
    }

    await update(panel, { [initial.property]: '' });
    await expectSlot(card, '');
    await update(panel, { [initial.property]: 'Updated details' });
    await expectSlot(card, 'Updated details');
    await update(panel, { [initial.property]: '' });
    await expectSlot(card, '');
  });
}

for (const mode of ['hydrated', 'client-created'] as const) {
  test(`${mode} text preserves nullish, falsy, whitespace, and mixed runs`, async ({ page }) => {
    const panel = await hydrate(page, mode);
    const card = panel.locator('#empty');
    await expectSlot(card, '');
    for (const [value, text] of [
      [null, ''], [undefined, ''], [0, '0'], [false, 'false'], [' ', ' '],
      ['Details', 'Details'], ['', ''],
    ] as const) {
      await update(panel, { details: value });
      await expectSlot(card, text);
      await expectSlot(panel.locator('#mixed'), `Before ${text} after`);
      await expectSlot(panel.locator('#whitespace'), ` ${text} `);
    }
  });

  test(`${mode} adjacent bindings retain their order around a conditional`, async ({ page }) => {
    const panel = await hydrate(page, mode);
    const card = panel.locator('#adjacent');
    const assignedText = () => card.evaluate(element =>
      element.shadowRoot!.querySelector('slot')!.assignedNodes().map(node => node.textContent));
    await expectSlot(card, '');
    await update(panel, { last: 'C' });
    expect(await assignedText()).toEqual(['C']);
    await update(panel, { middle: 'B', first: 'A' });
    expect(await assignedText()).toEqual(['A', 'B', 'C']);
    await update(panel, { first: '', middle: '', last: '' });
    await expectSlot(card, '');
    await update(panel, { showMiddle: false, middle: 'Removed' });
    await update(panel, { first: 'Left', last: 'Right' });
    expect(await assignedText()).toEqual(['Left', 'Right']);
    await update(panel, { showMiddle: true, middle: 'Middle' });
    expect(await assignedText()).toEqual(['Left', 'Middle', 'Right']);
    await update(panel, { first: '', middle: '', last: '' });
    await expectSlot(card, '');
  });

  test(`${mode} repeated text remains owned by its item across updates and removal`, async ({ page }) => {
    const panel = await hydrate(page, mode);
    const repeat = panel.locator('#repeat');
    await expectSlot(repeat, mode === 'hydrated' ? 'Second' : '');
    await update(panel, { items: [{ value: '' }, { value: 'Second' }] });
    await expectSlot(repeat, 'Second');
    await update(panel, { items: [{ value: 'First' }, { value: '' }] });
    await expectSlot(repeat, 'First');
    await update(panel, { items: [] });
    await expectSlot(repeat, '');
    await update(panel, { items: [{ value: 'Recreated' }] });
    await expectSlot(repeat, 'Recreated');
    await update(panel, { items: [{ value: '' }] });
    await expectSlot(repeat, '');
  });
}
