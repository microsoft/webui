// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Regression tests: custom elements inside conditional (<if>) blocks
 * that are initially false during SSR must mount correctly when the
 * condition flips true client-side.
 */

import { expect, test } from '@playwright/test';

test.describe('conditional-component', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/conditional-component/fixture.html');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-cond-parent') as any;
      return el && el.$ready === true;
    });
  });

  test('child component is NOT present when condition is false', async ({ page }) => {
    const result = await page.evaluate(() => {
      const parent = document.querySelector('test-cond-parent') as any;
      const child = parent.shadowRoot?.querySelector('test-child-comp');
      return { hasChild: !!child, show: parent.show };
    });

    expect(result.show).toBe(false);
    expect(result.hasChild).toBe(false);
  });

  test('child component mounts with shadow root when condition flips true', async ({ page }) => {
    await page.evaluate(() => {
      (document.querySelector('test-cond-parent') as any).show = true;
    });

    await page.waitForFunction(() => {
      const parent = document.querySelector('test-cond-parent') as any;
      const child = parent.shadowRoot?.querySelector('test-child-comp');
      return child && child.shadowRoot && child.$ready === true;
    }, null, { timeout: 2000 });

    const result = await page.evaluate(() => {
      const parent = document.querySelector('test-cond-parent') as any;
      const child = parent.shadowRoot?.querySelector('test-child-comp');
      return {
        hasShadowRoot: !!child?.shadowRoot,
        ready: child?.$ready,
        text: child?.shadowRoot?.querySelector('.child-text')?.textContent,
      };
    });

    expect(result.hasShadowRoot).toBe(true);
    expect(result.ready).toBe(true);
    expect(result.text).toBe('Child Active');
  });

  test('child component survives toggle off and back on', async ({ page }) => {
    await page.evaluate(() => {
      (document.querySelector('test-cond-parent') as any).show = true;
    });
    await page.waitForFunction(() => {
      const parent = document.querySelector('test-cond-parent') as any;
      const child = parent.shadowRoot?.querySelector('test-child-comp');
      return child && child.$ready === true;
    }, null, { timeout: 2000 });

    await page.evaluate(() => {
      (document.querySelector('test-cond-parent') as any).show = false;
    });
    await page.waitForFunction(() => {
      const parent = document.querySelector('test-cond-parent') as any;
      return !parent.shadowRoot?.querySelector('test-child-comp');
    }, null, { timeout: 2000 });

    await page.evaluate(() => {
      (document.querySelector('test-cond-parent') as any).show = true;
    });
    await page.waitForFunction(() => {
      const parent = document.querySelector('test-cond-parent') as any;
      const child = parent.shadowRoot?.querySelector('test-child-comp');
      return child && child.shadowRoot && child.$ready === true;
    }, null, { timeout: 2000 });

    const result = await page.evaluate(() => {
      const parent = document.querySelector('test-cond-parent') as any;
      const child = parent.shadowRoot?.querySelector('test-child-comp');
      return {
        text: child?.shadowRoot?.querySelector('.child-text')?.textContent,
      };
    });

    expect(result.text).toBe('Child Active');
  });

  test('nested: mid and grandchild components mount through two <if> layers', async ({ page }) => {
    await page.waitForFunction(() => {
      const el = document.querySelector('test-nested-cond-parent') as any;
      return el && el.$ready === true;
    });

    const before = await page.evaluate(() => {
      const parent = document.querySelector('test-nested-cond-parent') as any;
      return { hasMid: !!parent.shadowRoot?.querySelector('test-mid-comp') };
    });
    expect(before.hasMid).toBe(false);

    await page.evaluate(() => {
      (document.querySelector('test-nested-cond-parent') as any).show = true;
    });

    await page.waitForFunction(() => {
      const parent = document.querySelector('test-nested-cond-parent') as any;
      const mid = parent.shadowRoot?.querySelector('test-mid-comp');
      if (!mid || !mid.shadowRoot || !mid.$ready) return false;
      const grandchild = mid.shadowRoot.querySelector('test-grandchild-comp');
      return grandchild && grandchild.shadowRoot && grandchild.$ready === true;
    }, null, { timeout: 2000 });

    const result = await page.evaluate(() => {
      const parent = document.querySelector('test-nested-cond-parent') as any;
      const mid = parent.shadowRoot?.querySelector('test-mid-comp');
      const grandchild = mid?.shadowRoot?.querySelector('test-grandchild-comp');
      return {
        midReady: mid?.$ready,
        midText: mid?.shadowRoot?.querySelector('.mid-label')?.textContent,
        grandchildReady: grandchild?.$ready,
        grandchildText: grandchild?.shadowRoot?.querySelector('.grandchild-text')?.textContent,
      };
    });

    expect(result.midReady).toBe(true);
    expect(result.midText).toBe('Mid');
    expect(result.grandchildReady).toBe(true);
    expect(result.grandchildText).toBe('Grandchild Active');
  });

  test('click toggle button activates child via UI', async ({ page }) => {
    await page.locator('test-cond-parent').evaluateHandle((el) => {
      (el as any).shadowRoot.querySelector('.toggle').click();
    });

    await page.waitForFunction(() => {
      const parent = document.querySelector('test-cond-parent') as any;
      const child = parent.shadowRoot?.querySelector('test-child-comp');
      return child && child.shadowRoot && child.$ready === true;
    }, null, { timeout: 2000 });

    const result = await page.evaluate(() => {
      const parent = document.querySelector('test-cond-parent') as any;
      const child = parent.shadowRoot?.querySelector('test-child-comp');
      return {
        text: child?.shadowRoot?.querySelector('.child-text')?.textContent,
      };
    });

    expect(result.text).toBe('Child Active');
  });
});
