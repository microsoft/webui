// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Regression test for issue #379.
 *
 * When an observable is assigned at or before `super.connectedCallback()` to a
 * value that differs from the server-rendered DOM, the framework trusts the SSR
 * content (it does NOT reconcile) but emits a dev-facing hydration-mismatch
 * warning — the same signal React/Vue/Svelte/Solid produce. Assignments made
 * after `super.connectedCallback()`, deferred assignments, and pre-ready
 * assignments that match seeded SSR state must stay silent.
 */

import { expect, test } from '@playwright/test';

const MISMATCH = 'Hydration mismatch';

const ALL_TAGS = [
  'mismatch-field-default',
  'mismatch-constructor',
  'mismatch-before-super',
  'mismatch-after-super',
  'mismatch-deferred',
  'mismatch-seeded',
  'mismatch-whitespace',
] as const;

test.describe('hydration-mismatch (#379)', () => {
  let warnings: string[];

  test.beforeEach(async ({ page }) => {
    warnings = [];
    page.on('console', (msg) => {
      if (msg.type() === 'warning') warnings.push(msg.text());
    });

    await page.goto('/hydration-mismatch/fixture.html');

    await page.waitForFunction((tags) => {
      return tags.every((t) => (document.querySelector(t) as { $ready?: boolean } | null)?.$ready === true);
    }, ALL_TAGS as unknown as string[]);

    // The deferred component assigns in a post-ready task; wait for it to land.
    await page.waitForSelector('mismatch-deferred .content');
  });

  const warningsFor = (tag: string): string[] =>
    warnings.filter((w) => w.includes(MISMATCH) && w.includes(`<${tag}>`));

  test('warns exactly once per pre-ready timing and stays silent otherwise', async () => {
    await expect.poll(() => warnings.filter((w) => w.includes(MISMATCH)).length).toBe(3);

    expect(warningsFor('mismatch-field-default')).toHaveLength(1);
    expect(warningsFor('mismatch-constructor')).toHaveLength(1);
    expect(warningsFor('mismatch-before-super')).toHaveLength(1);

    expect(warningsFor('mismatch-after-super')).toHaveLength(0);
    expect(warningsFor('mismatch-deferred')).toHaveLength(0);
    expect(warningsFor('mismatch-seeded')).toHaveLength(0);
    expect(warningsFor('mismatch-whitespace')).toHaveLength(0);
  });

  test('multiline text agrees across compilation, SSR, hydration, slots, and live updates', async ({ page, request }) => {
    const warnings: string[] = [];
    const errors: string[] = [];
    page.on('console', (message) => {
      if (message.type() === 'warning') warnings.push(message.text());
    });
    page.on('pageerror', (error) => errors.push(error.message));

    const response = await request.get('/hydration-mismatch/fixture.html');
    const raw = await response.text();
    let releaseScript!: () => void;
    const scriptReady = new Promise<void>((resolve) => { releaseScript = resolve; });
    await page.route('**/dist/hydration-mismatch/element.js', async (route) => {
      await scriptReady;
      await route.continue();
    });
    const navigation = page.goto('/hydration-mismatch/fixture.html');
    await page.locator('mismatch-whitespace .native').waitFor();
    const before = await page.evaluate(() => {
      const host = document.querySelector('mismatch-whitespace')!;
      const root = host.shadowRoot!;
      const native = root.querySelector('.native')!;
      const wrapper = root.querySelector('whitespace-slot')!;
      const text = native.firstChild;
      const slot = wrapper.shadowRoot!.querySelector('slot')!;
      Object.assign(window, { whitespaceNodes: { host, native, wrapper, text, slot } });
      return {
        native: native.textContent,
        wrapper: wrapper.textContent,
        assigned: slot.assignedNodes().map((node) => node.textContent),
        nativeTypes: Array.from(native.childNodes, (node) => node.nodeType),
        wrapperTypes: Array.from(wrapper.childNodes, (node) => node.nodeType),
      };
    });
    releaseScript();
    await navigation;
    await page.waitForFunction(() =>
      (document.querySelector('mismatch-whitespace') as { $ready?: boolean } | null)?.$ready === true);
    // Wait for the existing genuine-mismatch diagnostic's asynchronous module.
    await expect.poll(() => warnings.filter((warning) => warning.includes(MISMATCH)).length).toBeGreaterThanOrEqual(3);

    const title = '\n      Downloads\n    ';
    const details = '\n              Open file\n            ';
    expect(raw).toContain(`<button class="native">${title}</button>`);
    expect(before).toEqual({
      native: title,
      wrapper: details,
      assigned: [details],
      nativeTypes: [3],
      wrapperTypes: [3],
    });
    const snapshot = () => page.evaluate(() => {
      const host = document.querySelector('mismatch-whitespace') as HTMLElement & {
        headerTitle: string;
        details: string;
        padded: string;
      };
      const root = host.shadowRoot!;
      const saved = (window as Window & {
        whitespaceNodes?: { host: Element; native: Element; wrapper: Element; text: Node; slot: HTMLSlotElement };
      }).whitespaceNodes!;
      const wrapper = root.querySelector('whitespace-slot')!;
      const slot = wrapper.shadowRoot!.querySelector('slot')!;
      return {
        state: { headerTitle: host.headerTitle, details: host.details, padded: host.padded },
        native: root.querySelector('.native')!.textContent,
        wrapper: wrapper.textContent,
        data: root.querySelector('.data')!.textContent,
        inline: root.querySelector('.inline')!.textContent,
        pre: root.querySelector('pre')!.textContent,
        textarea: root.querySelector('textarea')!.textContent,
        multilinePre: root.querySelector('pre.leading-newline')!.textContent,
        multilineTextarea: root.querySelector('textarea.leading-newline')!.textContent,
        conditional: root.querySelector('.conditional')!.textContent,
        repeat: root.querySelector('.repeat')!.textContent,
        assigned: slot.assignedNodes().map((node) => node.textContent),
        identity: host === saved.host
          && root.querySelector('.native') === saved.native
          && saved.native.firstChild === saved.text
          && wrapper === saved.wrapper && slot === saved.slot,
      };
    });
    expect(await snapshot()).toEqual({
      state: { headerTitle: 'Downloads', details: 'Open file', padded: '  data \n ' },
      native: title,
      wrapper: details,
      data: '  data \n ',
      inline: 'left Open file right',
      pre: '  Open file\n    tail  ',
      textarea: '  Open file\n    tail  ',
      multilinePre: '    Open file\n  ',
      multilineTextarea: '    Open file\n  ',
      conditional: '\n    Open file\n  ',
      repeat: '\n    First\n  \n    Second\n  ',
      assigned: [details],
      identity: true,
    });
    expect(warnings.filter((warning) => warning.includes('<mismatch-whitespace>'))).toEqual([]);
    expect(warnings.filter((warning) => warning.includes(MISMATCH))).toHaveLength(3);
    expect(errors).toEqual([]);

    const created = await page.evaluate(() => {
      const host = document.createElement('mismatch-whitespace');
      Object.assign(host, {
        headerTitle: 'Downloads', details: 'Open file', padded: '  data \n ', items: ['First', 'Second'],
      });
      document.body.append(host);
      const root = host.shadowRoot!;
      const texts = [
        '.native', 'whitespace-slot', '.data', '.inline', 'pre', 'textarea',
        'pre.leading-newline', 'textarea.leading-newline', '.conditional', '.repeat',
      ]
        .map((selector) => root.querySelector(selector)!.textContent);
      host.remove();
      return texts;
    });
    expect(created).toEqual([
      title, details, '  data \n ', 'left Open file right', '  Open file\n    tail  ',
      '  Open file\n    tail  ', '    Open file\n  ', '    Open file\n  ',
      '\n    Open file\n  ', '\n    First\n  \n    Second\n  ',
    ]);

    for (const value of ['  next \n ', '', '\n  ']) {
      await page.evaluate((value) => {
        const host = document.querySelector('mismatch-whitespace') as HTMLElement & {
            headerTitle: string; details: string; padded: string; items: string[];
        };
        host.headerTitle = value;
        host.details = value;
        host.padded = value;
        host.items = [value];
      }, value);
      await expect.poll(snapshot).toEqual({
        state: { headerTitle: value, details: value, padded: value },
        native: `\n      ${value}\n    `,
        wrapper: `\n              ${value}\n            `,
        data: value,
        inline: `left ${value} right`,
        pre: `  ${value}\n    tail  `,
        textarea: `  ${value}\n    tail  `,
        multilinePre: `    ${value}\n  `,
        multilineTextarea: `    ${value}\n  `,
        conditional: value ? `\n    ${value}\n  ` : '',
        repeat: `\n    ${value}\n  `,
        assigned: [`\n              ${value}\n            `],
        identity: true,
      });
    }
  });

  test('a genuinely different attribute value still warns for multiline text', async ({ page }) => {
    const mismatches: string[] = [];
    page.on('console', (message) => {
      if (message.type() === 'warning' && message.text().includes('<mismatch-whitespace>')) {
        mismatches.push(message.text());
      }
    });
    await page.route('**/hydration-mismatch/fixture.html', async (route) => {
      const response = await route.fetch();
      const body = (await response.text()).replace('details="Open file"', 'details="Client override"');
      await route.fulfill({ response, body });
    });
    await page.goto('/hydration-mismatch/fixture.html');
    await expect.poll(() => mismatches.length).toBe(1);
    expect(mismatches[0]).toContain('"details" changed at or before super.connectedCallback()');
    const state = await page.locator('mismatch-whitespace').evaluate((host) =>
      (host as HTMLElement & { details: string }).details);
    expect(state).toBe('Client override');
    expect(await page.locator('mismatch-whitespace whitespace-slot').textContent())
      .toBe('\n              Open file\n            ');
  });

  test('the warning names the mismatched observables', async () => {
    await expect.poll(() => warningsFor('mismatch-constructor').length).toBe(1);
    const [warning] = warningsFor('mismatch-constructor');
    expect(warning).toContain('"show"');
    expect(warning).toContain('"value"');
    expect(warning).toContain('super.connectedCallback()');
  });

  test('SSR DOM is trusted, not reconciled, for pre-ready mismatches', async ({ page }) => {
    for (const tag of ['mismatch-field-default', 'mismatch-constructor', 'mismatch-before-super']) {
      // Conditional content stays absent (server rendered it empty).
      expect(await page.locator(`${tag} .content`).count()).toBe(0);
      // Bound attribute keeps the server value, not the client "3".
      expect(await page.locator(`${tag} .box`).getAttribute('data-value')).not.toBe('3');
    }
  });

  test('element state holds the client value even though the DOM does not (the #379 divergence)', async ({ page }) => {
    const state = await page.evaluate(() => {
      const el = document.querySelector('mismatch-field-default') as { show?: boolean; value?: string } | null;
      return { show: el?.show, value: el?.value };
    });
    expect(state).toEqual({ show: true, value: '3' });
    expect(await page.locator('mismatch-field-default .content').count()).toBe(0);
  });

  test('post-super and deferred assignments update the DOM', async ({ page }) => {
    for (const tag of ['mismatch-after-super', 'mismatch-deferred']) {
      await expect(page.locator(`${tag} .content`)).toHaveText('CONTENT');
      expect(await page.locator(`${tag} .box`).getAttribute('data-value')).toBe('3');
    }
  });

  test('super.connectedCallback hydrates synchronously while the document is loading', async ({ page }) => {
    const lifecycle = await page.evaluate(() => {
      const el = document.querySelector('mismatch-after-super') as {
        readyStateAtConnect?: string;
        referencesReadyAfterSuper?: boolean;
      } | null;
      return {
        readyStateAtConnect: el?.readyStateAtConnect,
        referencesReadyAfterSuper: el?.referencesReadyAfterSuper,
      };
    });

    expect(lifecycle).toEqual({
      readyStateAtConnect: 'loading',
      referencesReadyAfterSuper: true,
    });
  });

  test('seeded pre-ready values match the server render and do not warn', async ({ page }) => {
    await expect(page.locator('mismatch-seeded .content')).toHaveText('CONTENT');
    expect(await page.locator('mismatch-seeded .box').getAttribute('data-value')).toBe('3');
    expect(warningsFor('mismatch-seeded')).toHaveLength(0);
  });
});
