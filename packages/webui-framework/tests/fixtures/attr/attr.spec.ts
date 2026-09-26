// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('attr fixture', () => {
  test('creating an element does not run property effects in the constructor', async ({ page }) => {
    await page.goto('/attr/fixture.html');
    const result = await page.evaluate(() => {
      const host = document.createElement('test-attr');
      const constructorAttributes = host.getAttributeNames();
      const upgraded = host instanceof customElements.get('test-attr')!;
      document.body.appendChild(host);
      return { constructorAttributes, upgraded, role: host.getAttribute('role') };
    });
    expect(result).toEqual({ constructorAttributes: [], upgraded: true, role: 'status' });
  });

  test('reconciles initial properties after refs and internals exist', async ({ page }) => {
    await page.goto('/attr/fixture.html');
    const initial = await page.locator('test-attr').first().evaluate((host) =>
      (host as unknown as { changeLog: { first: boolean; keys: string[]; refReady: boolean; internalsReady: boolean }[] }).changeLog,
    );
    expect(initial).toHaveLength(1);
    expect(initial[0]).toMatchObject({
      first: true,
      refReady: true,
      internalsReady: true,
    });
    expect(initial[0].keys).toContain('isActive');
  });

  test('reflects synchronously but coalesces net property effects', async ({ page }) => {
    await page.goto('/attr/fixture.html');
    const result = await page.locator('test-attr').first().evaluate(async (host) => {
      const el = host as HTMLElement & {
        label: string;
        isActive: boolean;
        changeLog: { first: boolean; keys: string[]; previousLabel: unknown }[];
      };
      const initialCount = el.changeLog.length;
      el.label = 'Intermediate';
      el.label = 'Final';
      el.isActive = true;
      const attributeImmediately = el.getAttribute('label');
      const boolImmediately = el.hasAttribute('is-active');
      const callbacksImmediately = el.changeLog.length;
      await new Promise<void>((resolve) => queueMicrotask(resolve));
      return {
        attributeImmediately, boolImmediately, callbacksImmediately,
        initialCount, newCalls: el.changeLog.slice(initialCount),
      };
    });
    expect(result.attributeImmediately).toBe('Final');
    expect(result.boolImmediately).toBe(true);
    expect(result.callbacksImmediately).toBe(result.initialCount);
    expect(result.newCalls).toEqual([{
      first: false,
      keys: ['label', 'isActive'],
      previousLabel: 'Status',
      refReady: true,
      internalsReady: true,
    }]);
  });

  test('queues writes made inside propertiesChanged for the next transaction', async ({ page }) => {
    await page.goto('/attr/fixture.html');
    const result = await page.locator('test-attr').first().evaluate(async (host) => {
      const el = host as HTMLElement & {
        label: string;
        displayValue: string;
        changeLog: { first: boolean; keys: string[] }[];
        propertiesChanged(changes: ReadonlyMap<string, unknown>, first: boolean): void;
      };
      const proto = Object.getPrototypeOf(el) as typeof el;
      const original = proto.propertiesChanged;
      proto.propertiesChanged = function (changes, first) {
        original.call(this, changes, first);
        if (changes.has('label')) this.displayValue = 'From callback';
      };
      try {
        const initialCount = el.changeLog.length;
        el.label = 'Changed';
        await new Promise<void>((resolve) => setTimeout(resolve, 0));
        return {
          calls: el.changeLog.slice(initialCount).map(({ first, keys }) => ({ first, keys })),
          displayValue: el.displayValue,
        };
      } finally {
        proto.propertiesChanged = original;
      }
    });
    expect(result).toEqual({
      calls: [
        { first: false, keys: ['label'] },
        { first: false, keys: ['displayValue'] },
      ],
      displayValue: 'From callback',
    });
  });

  test('does not rerun initial effects on move or lose changes while disconnected', async ({ page }) => {
    await page.goto('/attr/fixture.html');
    const result = await page.locator('test-attr').first().evaluate(async (host) => {
      const el = host as HTMLElement & {
        label: string;
        changeLog: { first: boolean; keys: string[] }[];
      };
      el.remove();
      el.label = 'Detached';
      await new Promise<void>((resolve) => queueMicrotask(resolve));
      const whileDetached = el.changeLog.length;
      document.body.appendChild(el);
      await new Promise<void>((resolve) => queueMicrotask(resolve));
      return {
        whileDetached,
        calls: el.changeLog.map(({ first, keys }) => ({ first, keys })),
        role: el.getAttribute('role'),
      };
    });
    expect(result.whileDetached).toBe(1);
    expect(result.calls).toEqual([
      { first: true, keys: ['label', 'displayValue', 'ctaHref', 'isActive', 'itemId', 'tag'] },
      { first: false, keys: ['label'] },
    ]);
    expect(result.role).toBe('status');
  });

  test('preserves an own property set before custom-element upgrade', async ({ page }) => {
    await page.addInitScript(() => {
      const host = document.createElement('test-attr') as HTMLElement & { label: string };
      host.label = 'Before upgrade';
      (window as Window & { preUpgradeHost?: HTMLElement }).preUpgradeHost = host;
    });
    await page.goto('/attr/fixture.html');
    const result = await page.evaluate(() => {
      const host = (window as Window & { preUpgradeHost?: HTMLElement }).preUpgradeHost!;
      document.body.appendChild(host);
      return {
        property: (host as HTMLElement & { label: string }).label,
        attribute: host.getAttribute('label'),
        upgraded: host instanceof customElements.get('test-attr')!,
      };
    });
    expect(result).toEqual({
      property: 'Before upgrade', attribute: 'Before upgrade', upgraded: true,
    });
  });

  test('rejects old per-property callbacks in development', async ({ page }) => {
    await page.goto('/attr/fixture.html');
    const error = page.waitForEvent('pageerror');
    await page.evaluate(() => {
      const ctor = customElements.get('test-attr')!;
      (ctor.prototype as HTMLElement & { labelChanged?: () => void }).labelChanged = () => {};
      document.body.appendChild(document.createElement('test-attr'));
    });
    expect((await error).message).toContain(
      'labelChanged is no longer called. Use propertiesChanged',
    );
  });

  test.beforeEach(async ({ page }) => {
    await page.goto('/attr/fixture.html');
    await page.waitForSelector('test-attr');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-attr');
      return el && (el as any).$ready === true;
    });
  });

  test('renders attribute-backed SSR text', async ({ page }) => {
    await expect(page.locator('test-attr .label')).toHaveText('Status');
    await expect(page.locator('test-attr .display')).toHaveText('Ready');
    await expect(page.locator('test-attr')).toHaveAttribute('label', 'Status');
  });

  test('updates default attribute names reactively', async ({ page }) => {
    await page.evaluate(() => {
      document.querySelector('test-attr')?.setAttribute('label', 'Mode');
    });

    await expect(page.locator('test-attr .label')).toHaveText('Mode');
  });

  test('updates custom attribute names reactively', async ({ page }) => {
    await page.evaluate(() => {
      document.querySelector('test-attr')?.setAttribute('display-value', 'Paused');
    });

    await expect(page.locator('test-attr .display')).toHaveText('Paused');
  });

  test('reacts to direct property updates', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.querySelector('test-attr') as { label: string; displayValue: string } | null;
      if (host) {
        host.label = 'Phase';
        host.displayValue = 'Running';
      }
    });

    await expect(page.locator('test-attr .label')).toHaveText('Phase');
    await expect(page.locator('test-attr .display')).toHaveText('Running');
  });

  test('reflects direct @attr property updates to host attributes', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.querySelector('test-attr') as { label: string; displayValue: string } | null;
      if (host) {
        host.label = 'bob';
        host.displayValue = 'Visible';
      }
    });

    await expect(page.locator('test-attr')).toHaveAttribute('label', 'bob');
    await expect(page.locator('test-attr')).toHaveAttribute('display-value', 'Visible');
    await expect(page.locator('test-attr .label')).toHaveText('bob');
    await expect(page.locator('test-attr .display')).toHaveText('Visible');
  });

  test('reflects @attr values applied through setState', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.querySelector('test-attr') as {
        label: string;
        displayValue: string;
        setState(state: Record<string, unknown>): void;
      } | null;
      host?.setState({ label: 'State Label', displayValue: 'State Value' });
    });

    await expect(page.locator('test-attr')).toHaveAttribute('label', 'State Label');
    await expect(page.locator('test-attr')).toHaveAttribute('display-value', 'State Value');
    await expect(page.locator('test-attr .label')).toHaveText('State Label');
    await expect(page.locator('test-attr .display')).toHaveText('State Value');
  });

  test('reflects @attr property values set before connection', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.createElement('test-attr') as HTMLElement & {
        label: string;
        displayValue: string;
      };
      host.id = 'dynamic-attr';
      host.label = 'Preconnect';
      host.displayValue = 'Before Append';
      document.body.appendChild(host);
    });
    await page.waitForFunction(() => {
      return (document.querySelector('#dynamic-attr') as any)?.$ready === true;
    });

    await expect(page.locator('#dynamic-attr')).toHaveAttribute('label', 'Preconnect');
    await expect(page.locator('#dynamic-attr')).toHaveAttribute('display-value', 'Before Append');
    await expect(page.locator('#dynamic-attr .label')).toHaveText('Preconnect');
    await expect(page.locator('#dynamic-attr .display')).toHaveText('Before Append');
  });

  test('keeps event markers from hijacking attr hydration targets', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.querySelector('test-attr') as { ctaHref: string } | null;
      if (host) {
        host.ctaHref = '/cart';
      }
    });

    await expect(page.locator('test-attr .cta')).toHaveAttribute('href', '/cart');
    await expect(page.locator('test-attr .logo')).toHaveAttribute('href', '/');
  });

  test('boolean attr defaults to false', async ({ page }) => {
    const active = await page.evaluate(() => {
      return (document.querySelector('test-attr') as any).isActive;
    });
    expect(active).toBe(false);
  });

  test('boolean attr becomes true when attribute is set', async ({ page }) => {
    await page.evaluate(() => {
      document.querySelector('test-attr')!.setAttribute('is-active', '');
    });

    const active = await page.evaluate(() => {
      return (document.querySelector('test-attr') as any).isActive;
    });
    expect(active).toBe(true);
  });

  test('boolean attr becomes false when attribute is removed', async ({ page }) => {
    await page.evaluate(() => {
      const el = document.querySelector('test-attr')!;
      el.setAttribute('is-active', '');
      el.removeAttribute('is-active');
    });

    const active = await page.evaluate(() => {
      return (document.querySelector('test-attr') as any).isActive;
    });
    expect(active).toBe(false);
  });

  test('boolean attr updates template bindings', async ({ page }) => {
    await expect(page.locator('test-attr .bool-target')).not.toHaveAttribute('data-active');
    await expect(page.locator('test-attr')).not.toHaveAttribute('is-active');

    await page.evaluate(() => {
      (document.querySelector('test-attr') as any).isActive = true;
    });

    await expect(page.locator('test-attr')).toHaveAttribute('is-active', '');
    await expect(page.locator('test-attr .bool-target')).toHaveAttribute('data-active', '');

    await page.evaluate(() => {
      (document.querySelector('test-attr') as any).isActive = false;
    });

    await expect(page.locator('test-attr')).not.toHaveAttribute('is-active');
    await expect(page.locator('test-attr .bool-target')).not.toHaveAttribute('data-active');
  });

  test('boolean attr sets checkbox checked property', async ({ page }) => {
    await expect(page.locator('test-attr .bool-check')).not.toBeChecked();

    await page.evaluate(() => {
      (document.querySelector('test-attr') as any).isActive = true;
    });

    await expect(page.locator('test-attr .bool-check')).toBeChecked();

    await page.evaluate(() => {
      (document.querySelector('test-attr') as any).isActive = false;
    });

    await expect(page.locator('test-attr .bool-check')).not.toBeChecked();
  });

  test('mixed static+dynamic attribute renders correctly', async ({ page }) => {
    await expect(page.locator('test-attr .mixed')).toHaveAttribute('href', '/items/42');

    await page.evaluate(() => {
      (document.querySelector('test-attr') as any).itemId = '99';
    });

    await expect(page.locator('test-attr .mixed')).toHaveAttribute('href', '/items/99');
  });

  test('mixed attribute with prefix and suffix', async ({ page }) => {
    await expect(page.locator('test-attr .mixed-class')).toHaveAttribute('data-tag', 'prefix-demo-suffix');

    await page.evaluate(() => {
      (document.querySelector('test-attr') as any).tag = 'live';
    });

    await expect(page.locator('test-attr .mixed-class')).toHaveAttribute('data-tag', 'prefix-live-suffix');
  });
});
