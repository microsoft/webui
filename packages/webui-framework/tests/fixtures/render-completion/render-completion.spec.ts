// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FLUSH_UPDATES } from '../../../src/render-scheduler.js';
import { expect, test, type Page } from '@playwright/test';

type RenderHost = HTMLElement & {
  value: string;
  $whenRendered(): Promise<void>;
  [FLUSH_UPDATES](): void;
  captureInChanged: boolean;
  throwInChanged: boolean;
  pending: Promise<void> | undefined;
  rows: Array<{ id: string; value: string }>;
  complexRows: Array<{ id: string; value: string }>;
  showUnloaded: boolean;
  note: string;
  html: string;
};

test('a reactive assignment and completion are sufficient to focus a revealed ref', async ({ page }) => {
  await page.goto('/render-completion/fixture.html');
  await expect(page.getByRole('button', { name: 'Continue', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Show action', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Continue', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Continue', exact: true })).toBeFocused();
});

test('only completion is exposed as a public rendering control', async ({ page }) => {
  await page.goto('/render-completion/fixture.html');
  const controls = await page.evaluate(() => {
    const root = document.querySelector<RenderHost>('test-completion-root')!;
    return {
      completion: typeof root.$whenRendered,
      update: '$update' in root,
      flush: '$flushUpdates' in root,
    };
  });
  expect(controls).toEqual({ completion: 'function', update: false, flush: false });
});

test('one completion includes affected nested Shadow and Light updates', async ({ page }) => {
  await page.goto('/render-completion/fixture.html');
  const result = await page.evaluate(async () => {
    const root = document.querySelector<RenderHost>('test-completion-root')!;
    root.value = '7';
    await root.$whenRendered();
    const child = root.shadowRoot!.querySelector('test-completion-child')!;
    return {
      child: child.shadowRoot!.querySelector('.value')!.textContent,
      leaf: child.shadowRoot!.querySelector('.action')!.textContent,
      disabled: child.shadowRoot!.querySelector<HTMLButtonElement>('.action')!.disabled,
    };
  });
  expect(result).toEqual({ child: '8', leaf: '8', disabled: false });
});

test('a pending descendant is captured even when the root has no update', async ({ page }) => {
  await page.goto('/render-completion/fixture.html');
  const value = await page.evaluate(async () => {
    const root = document.querySelector<RenderHost>('test-completion-root')!;
    const child = root.shadowRoot!.querySelector<RenderHost>('test-completion-child')!;
    child.value = '12';
    await root.$whenRendered();
    return child.shadowRoot!.querySelector('.action')!.textContent;
  });
  expect(value).toBe('13');
});

for (const observed of [false, true]) {
  test(`complex parent bindings coalesce into one child render with completion=${observed}`, async ({ page }) => {
    await page.goto('/render-completion/fixture.html');
    const result = await page.evaluate(async observed => {
      const root = document.querySelector<RenderHost>('test-completion-root')!;
      root.rows = [];
      root.complexRows = [{ id: 'complex', value: '1' }];
      await root.$whenRendered();
      const child = root.shadowRoot!.querySelector<RenderHost>('[data-row="complex"]')!;
      let prototype = Object.getPrototypeOf(child) as object | null;
      let render: symbol | undefined;
      while (prototype && !render) {
        render = Object.getOwnPropertySymbols(prototype).find(key => key.description === 'render');
        prototype = Object.getPrototypeOf(prototype) as object | null;
      }
      if (!render) throw new Error('render hook not found');
      const target = child as RenderHost & Record<symbol, () => void>;
      const original = target[render];
      let renders = 0;
      target[render] = function (): void {
        renders++;
        original.call(this);
      };
      try {
        root.complexRows = [{ id: 'complex', value: '7' }];
        if (observed) await root.$whenRendered();
        else {
          await Promise.resolve();
          await Promise.resolve();
          await Promise.resolve();
        }
        return {
          renders,
          label: child.shadowRoot!.querySelector('.label')!.textContent,
          leaf: child.shadowRoot!.querySelector('.action')!.textContent,
        };
      } finally {
        delete target[render];
      }
    }, observed);
    expect(result).toEqual({ renders: 1, label: '7', leaf: '8' });
  });
}

test('a request from Changed includes the setter trailing that callback', async ({ page }) => {
  await page.goto('/render-completion/fixture.html');
  const value = await page.evaluate(async () => {
    const root = document.querySelector<RenderHost>('test-completion-root')!;
    root.captureInChanged = true;
    root.value = '20';
    await root.pending;
    const child = root.shadowRoot!.querySelector('test-completion-child')!;
    return child.shadowRoot!.querySelector('.action')!.textContent;
  });
  expect(value).toBe('21');
});

test('the first boundary can open inside an otherwise unobserved parent render', async ({ page }) => {
  await page.goto('/render-completion/fixture.html');
  const value = await page.evaluate(async () => {
    const root = document.querySelector<RenderHost>('test-completion-root')!;
    const child = root.shadowRoot!.querySelector<RenderHost>('test-completion-child')!;
    child.captureInChanged = true;
    root.value = '50';
    await Promise.resolve();
    if (!child.pending) throw new Error('the parent render did not open a child boundary');
    await child.pending;
    return child.shadowRoot!.querySelector('.action')!.textContent;
  });
  expect(value).toBe('51');
});

test('callback failure rejects a requested boundary without swallowing the error', async ({ page }) => {
  await page.goto('/render-completion/fixture.html');
  const result = await page.evaluate(async () => {
    const root = document.querySelector<RenderHost>('test-completion-root')!;
    root.captureInChanged = true;
    root.throwInChanged = true;
    let thrown: unknown;
    try { root.value = '20'; } catch (error) { thrown = error; }
    try {
      await root.pending;
      return false;
    } catch (error) {
      return error === thrown && error instanceof Error && error.message === 'test callback failed';
    }
  });
  expect(result).toBe(true);
});

test('slotted descendant updates are included', async ({ page }) => {
  await page.goto('/render-completion/fixture.html');
  const result = await page.evaluate(async () => {
    const root = document.querySelector<RenderHost>('test-completion-root')!;
    const child = document.createElement('test-completion-child') as RenderHost;
    root.appendChild(child);
    child.value = '30';
    await root.$whenRendered();
    return {
      slotted: child.assignedSlot !== null,
      value: child.shadowRoot!.querySelector('.action')!.textContent,
    };
  });
  expect(result).toEqual({ slotted: true, value: '31' });
});

test('list removal permits application-selected focus after descendant completion', async ({ page }) => {
  await page.goto('/render-completion/fixture.html');
  const result = await page.evaluate(async () => {
    const root = document.querySelector<RenderHost>('test-completion-root')!;
    const removed = root.shadowRoot!.querySelector<RenderHost>('[data-row="b"]')!;
    removed.shadowRoot!.querySelector<HTMLButtonElement>('.action')!.focus();
    removed.value = '40';
    root.rows = [{ id: 'a', value: '1' }, { id: 'c', value: '5' }];
    await root.$whenRendered();
    const target = root.shadowRoot!.querySelector('[data-row="c"]')!;
    const button = target.shadowRoot!.querySelector<HTMLButtonElement>('.action')!;
    button.focus();
    return {
      ids: Array.from(root.shadowRoot!.querySelectorAll('[data-row]'), el => el.getAttribute('data-row')),
      value: button.textContent,
      focused: target.shadowRoot!.activeElement === button,
    };
  });
  expect(result).toEqual({ ids: ['a', 'c'], value: '6', focused: true });
});

test('completion does not move focus that the user moved elsewhere', async ({ page }) => {
  await page.goto('/render-completion/fixture.html');
  const focused = await page.evaluate(async () => {
    const root = document.querySelector<RenderHost>('test-completion-root')!;
    root.rows = [{ id: 'c', value: '5' }];
    const waiting = root.$whenRendered();
    const outside = document.querySelector<HTMLButtonElement>('#outside')!;
    outside.focus();
    await waiting;
    return document.activeElement === outside;
  });
  expect(focused).toBe(true);
});

test('raw HTML disposal retires captured WebUI work', async ({ page }) => {
  await page.goto('/render-completion/fixture.html');
  const removed = await page.evaluate(async () => {
    const root = document.querySelector<RenderHost>('test-completion-root')!;
    root.html = '<test-completion-child value="2"></test-completion-child>';
    await root.$whenRendered();
    const child = root.shadowRoot!.querySelector<RenderHost>('.raw test-completion-child')!;
    root.html = '';
    child.value = '10';
    await root.$whenRendered();
    return !child.isConnected && root.shadowRoot!.querySelector('.raw')!.textContent === '';
  });
  expect(removed).toBe(true);
});

test('external removal rejects a root request without stealing focus', async ({ page }) => {
  await page.goto('/render-completion/fixture.html');
  const rejected = await page.evaluate(async () => {
    const root = document.querySelector<RenderHost>('test-completion-root')!;
    root.value = '2';
    const waiting = root.$whenRendered();
    root.remove();
    try { await waiting; return false; } catch (error) {
      return error instanceof DOMException && error.name === 'AbortError';
    }
  });
  expect(rejected).toBe(true);
});

test('newly affected unloaded work rejects, and a later ready boundary succeeds', async ({ page }) => {
  await page.goto('/render-completion/fixture.html');
  const result = await page.evaluate(async () => {
    const root = document.querySelector<RenderHost>('test-completion-root')!;
    root.showUnloaded = true;
    let rejected = false;
    try { await root.$whenRendered(); } catch (error) {
      rejected = error instanceof DOMException && error.name === 'InvalidStateError';
    }
    window.__defineCompletionUnloaded!();
    await root.$whenRendered();
    return {
      rejected,
      text: root.shadowRoot!.querySelector('test-completion-unloaded span')!.textContent,
    };
  });
  expect(result).toEqual({ rejected: true, text: 'loaded' });
});

test('untouched lazy descendants do not wait; retained affected writes reject', async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(window, 'IntersectionObserver', {
      value: class {
        observe(): void {}
        unobserve(): void {}
        disconnect(): void {}
      },
    });
  });
  await page.goto('/render-completion/fixture.html');
  const rejected = await page.evaluate(async () => {
    const root = document.querySelector<RenderHost>('test-completion-root')!;
    await root.$whenRendered();
    const lazy = root.shadowRoot!.querySelector<RenderHost>('test-completion-lazy')!;
    lazy.note = 'changed while dormant';
    try {
      await root.$whenRendered();
      return false;
    } catch (error) {
      return error instanceof DOMException && error.name === 'InvalidStateError';
    }
  });
  expect(rejected).toBe(true);
});

async function guardStyles(page: Page): Promise<void> {
  await page.route('**/css-link/fixture.html', async route => {
    const response = await route.fetch();
    await route.fulfill({
      response,
      headers: {
        ...response.headers(),
        'content-security-policy':
          "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'",
      },
    });
  });
}

for (const failed of [false, true]) {
  test(`started stylesheet-gated mounts ${failed ? 'reject failures' : 'wait for DOM insertion'}`, async ({ page }) => {
    await guardStyles(page);
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    await page.route('**/child.css', async route => {
      await gate;
      if (failed) await route.fulfill({ status: 404, body: 'not found' });
      else await route.continue();
    });
    await page.goto('/css-link/fixture.html', { waitUntil: 'domcontentloaded' });
    await page.evaluate(() => {
      const root = document.querySelector<RenderHost>('test-link-host')!;
      root.shadowRoot!.querySelector('.slot')!.appendChild(document.createElement('test-link-child'));
      const state = window as unknown as {
        completionFinished: boolean;
        completionResult: Promise<{ failed: boolean; hasContent: boolean }>;
      };
      state.completionFinished = false;
      state.completionResult = root.$whenRendered().then(
        () => {
          state.completionFinished = true;
          const child = root.shadowRoot!.querySelector('test-link-child')!;
          return { failed: false, hasContent: child.shadowRoot!.querySelector('.child-label') !== null };
        },
        () => {
          state.completionFinished = true;
          return { failed: true, hasContent: false };
        },
      );
    });
    expect(await page.evaluate(() => (window as unknown as { completionFinished: boolean }).completionFinished)).toBe(false);
    release();
    const result = await page.evaluate(() => (window as unknown as {
      completionResult: Promise<{ failed: boolean; hasContent: boolean }>;
    }).completionResult);
    expect(result).toEqual({ failed, hasContent: !failed });
  });
}
