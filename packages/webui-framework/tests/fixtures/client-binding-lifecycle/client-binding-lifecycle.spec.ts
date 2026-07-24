// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test, type Page } from '@playwright/test';

type ChildSnapshot = {
  value: string | undefined;
  connectedValue: string;
  fallbackApplied: string;
  valueText: string | undefined;
};

function readChildSnapshot(page: Page, selector: string): Promise<ChildSnapshot | null> {
  return page.evaluate((hostSelector) => {
    const host = document.querySelector(hostSelector);
    const child = host?.shadowRoot?.querySelector('test-lifecycle-child') as
      | (HTMLElement & {
        value?: string;
        connectedValue?: string;
        fallbackApplied?: string;
        shadowRoot: ShadowRoot;
      })
      | null;
    if (!child) return null;
    return {
      value: child.value,
      connectedValue: child.connectedValue ?? '',
      fallbackApplied: child.fallbackApplied ?? '',
      valueText: child.shadowRoot?.querySelector('.value')?.textContent ?? undefined,
    };
  }, selector);
}

async function waitForChildValue(page: Page, selector: string, expected: string): Promise<void> {
  await page.waitForFunction(
    ({ hostSelector, value }) => {
      const host = document.querySelector(hostSelector);
      const child = host?.shadowRoot?.querySelector('test-lifecycle-child') as
        | (HTMLElement & { value?: string })
        | null;
      return child?.value === value;
    },
    { hostSelector: selector, value: expected },
  );
}

async function waitForConnectedValue(page: Page, selector: string, expected: string): Promise<void> {
  await page.waitForFunction(
    ({ hostSelector, value }) => {
      const host = document.querySelector(hostSelector);
      const child = host?.shadowRoot?.querySelector('test-lifecycle-child') as
        | (HTMLElement & { connectedValue?: string })
        | null;
      return child?.connectedValue === value;
    },
    { hostSelector: selector, value: expected },
  );
}

function readPositionalNestedSequence(page: Page, selector: string): Promise<string[]> {
  return page.evaluate((hostSelector) => {
    const host = document.querySelector(hostSelector);
    const children = host?.shadowRoot?.children ?? [];
    const sequence: string[] = [];
    for (let i = 0; i < children.length; i++) {
      const child = children[i];
      if (child.classList.contains('group-label')) {
        sequence.push(`group:${child.textContent ?? ''}`);
      } else if (child.classList.contains('section-label')) {
        sequence.push(`section:${child.textContent ?? ''}`);
      } else if (child.localName === 'test-lifecycle-child') {
        sequence.push(`child:${(child as HTMLElement & { value?: string }).value ?? ''}`);
      }
    }
    return sequence;
  }, selector);
}

test.describe('client binding lifecycle: CSR-created children', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/client-binding-lifecycle/fixture.html');
    await page.waitForFunction(() => {
      const el = document.querySelector('#ssr-parent-seed');
      return (el as HTMLElement & { $ready?: boolean } | null)?.$ready === true;
    });
  });

  test('child fallback from connectedCallback is not clobbered by an initially unset parent binding', async ({ page }) => {
    await page.evaluate(() => {
      const parent = document.createElement('test-lifecycle-parent');
      parent.id = 'dynamic-fallback';
      document.querySelector('#mount')?.appendChild(parent);
    });

    await waitForChildValue(page, '#dynamic-fallback', 'set-by-child');

    const snapshot = await readChildSnapshot(page, '#dynamic-fallback');
    expect(snapshot).toEqual({
      value: 'set-by-child',
      connectedValue: '<unset>',
      fallbackApplied: 'yes',
      valueText: 'set-by-child',
    });
  });

  test('initial parent binding is visible during child connectedCallback', async ({ page }) => {
    await page.evaluate(() => {
      const parent = document.createElement('test-lifecycle-parent') as HTMLElement & { val?: string };
      parent.id = 'dynamic-bound';
      parent.val = 'set-by-parent-before-connect';
      document.querySelector('#mount')?.appendChild(parent);
    });

    await waitForConnectedValue(page, '#dynamic-bound', 'set-by-parent-before-connect');

    const snapshot = await readChildSnapshot(page, '#dynamic-bound');
    expect(snapshot).toEqual({
      value: 'set-by-parent-before-connect',
      connectedValue: 'set-by-parent-before-connect',
      fallbackApplied: 'no',
      valueText: 'set-by-parent-before-connect',
    });
  });

  test('later parent updates still flow to a child that used its connectedCallback fallback', async ({ page }) => {
    await page.evaluate(() => {
      const parent = document.createElement('test-lifecycle-parent');
      parent.id = 'dynamic-live';
      document.querySelector('#mount')?.appendChild(parent);
    });
    await waitForChildValue(page, '#dynamic-live', 'set-by-child');

    await page.evaluate(() => {
      const parent = document.querySelector('#dynamic-live') as HTMLElement & {
        setParentValue(value: string): void;
      };
      parent.setParentValue('set-by-parent-after-connect');
    });

    await waitForChildValue(page, '#dynamic-live', 'set-by-parent-after-connect');

    const snapshot = await readChildSnapshot(page, '#dynamic-live');
    expect(snapshot).toEqual({
      value: 'set-by-parent-after-connect',
      connectedValue: '<unset>',
      fallbackApplied: 'yes',
      valueText: 'set-by-parent-after-connect',
    });
  });

  test('conditional-created child fallback is not clobbered by the first binding pass', async ({ page }) => {
    await page.evaluate(() => {
      const parent = document.createElement('test-lifecycle-conditional-parent') as HTMLElement & {
        showChild(): void;
      };
      parent.id = 'dynamic-conditional';
      document.querySelector('#mount')?.appendChild(parent);
      parent.showChild();
    });

    await waitForChildValue(page, '#dynamic-conditional', 'set-by-child');

    const snapshot = await readChildSnapshot(page, '#dynamic-conditional');
    expect(snapshot).toEqual({
      value: 'set-by-child',
      connectedValue: '<unset>',
      fallbackApplied: 'yes',
      valueText: 'set-by-child',
    });
  });

  test('repeat-created child fallback is not clobbered by the first binding pass', async ({ page }) => {
    await page.evaluate(() => {
      const parent = document.createElement('test-lifecycle-repeat-parent') as HTMLElement & {
        setItems(items: Array<{ id: string; value?: string }>): void;
      };
      parent.id = 'dynamic-repeat';
      document.querySelector('#mount')?.appendChild(parent);
      parent.setItems([{ id: 'missing' }]);
    });

    await waitForChildValue(page, '#dynamic-repeat', 'set-by-child');

    const snapshot = await readChildSnapshot(page, '#dynamic-repeat');
    expect(snapshot).toEqual({
      value: 'set-by-child',
      connectedValue: '<unset>',
      fallbackApplied: 'yes',
      valueText: 'set-by-child',
    });
  });

  test('conditional-created nested repeat children are inserted after detached first update', async ({ page }) => {
    await page.evaluate(() => {
      const parent = document.createElement('test-lifecycle-conditional-repeat-parent') as HTMLElement & {
        showItems(items: Array<{ id: string; value?: string }>): void;
      };
      parent.id = 'dynamic-conditional-repeat';
      document.querySelector('#mount')?.appendChild(parent);
      parent.showItems([{ id: 'missing' }]);
    });

    await waitForChildValue(page, '#dynamic-conditional-repeat', 'set-by-child');

    const snapshot = await readChildSnapshot(page, '#dynamic-conditional-repeat');
    expect(snapshot).toEqual({
      value: 'set-by-child',
      connectedValue: '<unset>',
      fallbackApplied: 'yes',
      valueText: 'set-by-child',
    });
  });

  test('repeat-created nested repeat children are inserted after detached first update', async ({ page }) => {
    await page.evaluate(() => {
      const parent = document.createElement('test-lifecycle-nested-repeat-parent') as HTMLElement & {
        setGroups(groups: Array<{ id: string; items: Array<{ id: string; value?: string }> }>): void;
      };
      parent.id = 'dynamic-nested-repeat';
      document.querySelector('#mount')?.appendChild(parent);
      parent.setGroups([{ id: 'group', items: [{ id: 'missing' }] }]);
    });

    await waitForChildValue(page, '#dynamic-nested-repeat', 'set-by-child');

    const snapshot = await readChildSnapshot(page, '#dynamic-nested-repeat');
    expect(snapshot).toEqual({
      value: 'set-by-child',
      connectedValue: '<unset>',
      fallbackApplied: 'yes',
      valueText: 'set-by-child',
    });
  });

  test('keyed groups move with their nested child instances', async ({ page }) => {
    await page.evaluate(() => {
      const parent = document.createElement('test-lifecycle-keyed-nested-repeat-parent') as HTMLElement & {
        setGroups(groups: Array<{ id: string; items: Array<{ id: string; value?: string }> }>): void;
      };
      parent.id = 'dynamic-keyed-nested-repeat';
      document.querySelector('#mount')?.appendChild(parent);
      parent.setGroups([
        { id: 'a', items: [{ id: 'a1', value: 'A1' }] },
        {
          id: 'b',
          items: [
            { id: 'b1', value: 'B1' },
            { id: 'b2', value: 'B2' },
          ],
        },
      ]);
    });

    await page.waitForFunction(() => {
      const parent = document.querySelector('#dynamic-keyed-nested-repeat');
      return parent?.shadowRoot?.querySelectorAll('test-lifecycle-child').length === 3;
    });
    await page.evaluate(() => {
      const parent = document.querySelector('#dynamic-keyed-nested-repeat');
      const root = parent?.shadowRoot;
      const win = window as unknown as {
        __groupA?: Element;
        __groupB?: Element;
        __childA?: Element;
        __childB1?: Element;
        __childB2?: Element;
      };
      win.__groupA = root?.querySelector('[data-id="a"]') ?? undefined;
      win.__groupB = root?.querySelector('[data-id="b"]') ?? undefined;
      const children = root?.querySelectorAll('test-lifecycle-child');
      win.__childA = children?.[0];
      win.__childB1 = children?.[1];
      win.__childB2 = children?.[2];
    });

    await page.evaluate(() => {
      const parent = document.querySelector('#dynamic-keyed-nested-repeat') as HTMLElement & {
        setGroups(groups: Array<{ id: string; items: Array<{ id: string; value?: string }> }>): void;
      };
      parent.setGroups([
        {
          id: 'b',
          items: [
            { id: 'b2', value: 'B2 updated' },
            { id: 'b1', value: 'B1 updated' },
          ],
        },
        { id: 'a', items: [{ id: 'a1', value: 'A1 updated' }] },
      ]);
    });

    await page.waitForFunction(() => {
      const parent = document.querySelector('#dynamic-keyed-nested-repeat');
      return parent?.shadowRoot?.querySelectorAll('test-lifecycle-child').length === 3;
    });
    const result = await page.evaluate(() => {
      const parent = document.querySelector('#dynamic-keyed-nested-repeat');
      const root = parent?.shadowRoot;
      const labels = root?.querySelectorAll('.group-label');
      const children = root?.querySelectorAll('test-lifecycle-child');
      const win = window as unknown as {
        __groupA?: Element;
        __groupB?: Element;
        __childA?: Element;
        __childB1?: Element;
        __childB2?: Element;
      };
      return {
        groupBPreserved: win.__groupB === labels?.[0],
        groupAPreserved: win.__groupA === labels?.[1],
        childB2Preserved: win.__childB2 === children?.[0],
        childB1Preserved: win.__childB1 === children?.[1],
        childAPreserved: win.__childA === children?.[2],
      };
    });

    expect(result).toEqual({
      groupBPreserved: true,
      groupAPreserved: true,
      childB2Preserved: true,
      childB1Preserved: true,
      childAPreserved: true,
    });
    const sequence = await readPositionalNestedSequence(
      page,
      '#dynamic-keyed-nested-repeat',
    );
    expect(sequence).toEqual([
      'group:b',
      'child:B2 updated',
      'child:B1 updated',
      'group:a',
      'child:A1 updated',
    ]);
  });

  test('removed nested items are not resurrected when a keyed parent moves', async ({ page }) => {
    await page.evaluate(() => {
      const parent = document.createElement('test-lifecycle-keyed-nested-repeat-parent') as HTMLElement & {
        setGroups(groups: Array<{ id: string; items: Array<{ id: string; value?: string }> }>): void;
      };
      parent.id = 'dynamic-keyed-nested-shrink';
      document.querySelector('#mount')?.appendChild(parent);
      parent.setGroups([
        { id: 'a', items: [{ id: 'a1', value: 'A1' }] },
        {
          id: 'b',
          items: [
            { id: 'b1', value: 'B1' },
            { id: 'b2', value: 'B2' },
          ],
        },
      ]);
    });

    await page.waitForFunction(() => {
      const parent = document.querySelector('#dynamic-keyed-nested-shrink');
      return parent?.shadowRoot?.querySelectorAll('test-lifecycle-child').length === 3;
    });
    await page.evaluate(() => {
      const parent = document.querySelector('#dynamic-keyed-nested-shrink') as HTMLElement & {
        setGroups(groups: Array<{ id: string; items: Array<{ id: string; value?: string }> }>): void;
      };
      const children = parent.shadowRoot?.querySelectorAll('test-lifecycle-child');
      (window as unknown as { __removedNestedChild?: Element }).__removedNestedChild = children?.[2];
      parent.setGroups([
        { id: 'a', items: [{ id: 'a1', value: 'A1' }] },
        { id: 'b', items: [{ id: 'b1', value: 'B1' }] },
      ]);
    });

    await page.waitForFunction(() => {
      const parent = document.querySelector('#dynamic-keyed-nested-shrink');
      return parent?.shadowRoot?.querySelectorAll('test-lifecycle-child').length === 2;
    });
    await page.evaluate(() => {
      const parent = document.querySelector('#dynamic-keyed-nested-shrink') as HTMLElement & {
        setGroups(groups: Array<{ id: string; items: Array<{ id: string; value?: string }> }>): void;
      };
      parent.setGroups([
        { id: 'b', items: [{ id: 'b1', value: 'B1 updated' }] },
        { id: 'a', items: [{ id: 'a1', value: 'A1 updated' }] },
      ]);
    });

    await page.waitForFunction(() => {
      const parent = document.querySelector('#dynamic-keyed-nested-shrink');
      const labels = parent?.shadowRoot?.querySelectorAll('.group-label');
      return labels?.[0]?.textContent === 'b';
    });
    const sequence = await readPositionalNestedSequence(
      page,
      '#dynamic-keyed-nested-shrink',
    );
    const removedConnected = await page.evaluate(
      () => (window as unknown as { __removedNestedChild?: Element })
        .__removedNestedChild?.isConnected,
    );

    expect(sequence).toEqual([
      'group:b',
      'child:B1 updated',
      'group:a',
      'child:A1 updated',
    ]);
    expect(removedConnected).toBe(false);
  });

  test('deep nested removals survive keyed ancestor movement', async ({ page }) => {
    await page.evaluate(() => {
      const parent = document.createElement(
        'test-lifecycle-deep-keyed-nested-repeat-parent',
      ) as HTMLElement & {
        setGroups(groups: Array<{
          id: string;
          sections: Array<{
            id: string;
            items: Array<{ id: string; value?: string }>;
          }>;
        }>): void;
      };
      parent.id = 'dynamic-deep-keyed-nested';
      document.querySelector('#mount')?.appendChild(parent);
      parent.setGroups([{
        id: 'g',
        sections: [
          { id: 'a', items: [{ id: 'a1', value: 'A1' }] },
          {
            id: 'b',
            items: [
              { id: 'b1', value: 'B1' },
              { id: 'b2', value: 'B2' },
            ],
          },
        ],
      }]);
    });

    await page.waitForFunction(() => {
      const parent = document.querySelector('#dynamic-deep-keyed-nested');
      return parent?.shadowRoot?.querySelectorAll('test-lifecycle-child').length === 3;
    });
    await page.evaluate(() => {
      const parent = document.querySelector('#dynamic-deep-keyed-nested') as HTMLElement & {
        setGroups(groups: Array<{
          id: string;
          sections: Array<{
            id: string;
            items: Array<{ id: string; value?: string }>;
          }>;
        }>): void;
      };
      const children = parent.shadowRoot?.querySelectorAll('test-lifecycle-child');
      (window as unknown as { __removedDeepChild?: Element }).__removedDeepChild = children?.[2];
      parent.setGroups([{
        id: 'g',
        sections: [
          { id: 'a', items: [{ id: 'a1', value: 'A1 updated' }] },
          { id: 'b', items: [{ id: 'b1', value: 'B1 updated' }] },
        ],
      }]);
    });

    await page.waitForFunction(() => {
      const parent = document.querySelector('#dynamic-deep-keyed-nested');
      return parent?.shadowRoot?.querySelectorAll('test-lifecycle-child').length === 2;
    });
    await page.evaluate(() => {
      const parent = document.querySelector('#dynamic-deep-keyed-nested') as HTMLElement & {
        setGroups(groups: Array<{
          id: string;
          sections: Array<{
            id: string;
            items: Array<{ id: string; value?: string }>;
          }>;
        }>): void;
      };
      parent.setGroups([{
        id: 'g',
        sections: [
          { id: 'b', items: [{ id: 'b1', value: 'B1 moved' }] },
          { id: 'a', items: [{ id: 'a1', value: 'A1 moved' }] },
        ],
      }]);
    });

    await page.waitForFunction(() => {
      const parent = document.querySelector('#dynamic-deep-keyed-nested');
      return parent?.shadowRoot?.querySelector('.section-label')?.textContent === 'b';
    });
    const sequence = await readPositionalNestedSequence(
      page,
      '#dynamic-deep-keyed-nested',
    );
    const removedConnected = await page.evaluate(
      () => (window as unknown as { __removedDeepChild?: Element })
        .__removedDeepChild?.isConnected,
    );

    expect(sequence).toEqual([
      'group:g',
      'section:b',
      'child:B1 moved',
      'section:a',
      'child:A1 moved',
    ]);
    expect(removedConnected).toBe(false);
  });

  test('reused positional items update nested repeats in place', async ({ page }) => {
    await page.evaluate(() => {
      const parent = document.createElement('test-lifecycle-positional-nested-repeat-parent') as HTMLElement & {
        setGroups(groups: Array<{ id: string; items: Array<{ id: string; value?: string }> }>): void;
      };
      parent.id = 'dynamic-positional-nested-repeat';
      document.querySelector('#mount')?.appendChild(parent);
      parent.setGroups([
        { id: 'a', items: [{ id: 'a1', value: 'A1' }] },
        { id: 'b', items: [{ id: 'b1', value: 'B1' }] },
      ]);
    });

    await page.waitForFunction(() => {
      const parent = document.querySelector('#dynamic-positional-nested-repeat');
      return parent?.shadowRoot?.querySelectorAll('test-lifecycle-child').length === 2;
    });

    await page.evaluate(() => {
      const parent = document.querySelector('#dynamic-positional-nested-repeat') as HTMLElement & {
        setGroups(groups: Array<{ id: string; items: Array<{ id: string; value?: string }> }>): void;
      };
      parent.setGroups([
        { id: 'b', items: [{ id: 'b1', value: 'B1' }, { id: 'b2', value: 'B2' }] },
        { id: 'a', items: [{ id: 'a1', value: 'A1' }] },
      ]);
    });

    await page.waitForFunction(() => {
      const parent = document.querySelector('#dynamic-positional-nested-repeat');
      return parent?.shadowRoot?.querySelectorAll('test-lifecycle-child').length === 3;
    });

    const sequence = await readPositionalNestedSequence(
      page,
      '#dynamic-positional-nested-repeat',
    );
    expect(sequence).toEqual(['group:b', 'child:B1', 'child:B2', 'group:a', 'child:A1']);
  });
});
