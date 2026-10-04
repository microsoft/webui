// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.goto('/template-owner-document/fixture.html');
  await page.waitForFunction(() =>
    customElements.get('test-owner-shadow') !== undefined
    && customElements.get('test-owner-light') !== undefined
    && customElements.get('test-owner-probe') !== undefined);
});

test('clones cached template descendants into the requested document', async ({ page }) => {
  expect(await page.evaluate(() => window.inspectTemplateCloneDocuments())).toEqual({
    localFragment: true,
    localChildren: true,
    foreignFragment: true,
    foreignChildren: true,
  });
});

for (const tag of ['test-owner-shadow', 'test-owner-light']) {
  test(`${tag} preserves owner document and custom-element timing`, async ({ page }) => {
    const result = await page.evaluate((tag) => {
      const host = document.createElement(tag);
      document.body.appendChild(host);
      const root = host.shadowRoot ?? host;
      const media = Array.from(root.querySelectorAll('video'));
      const probes = Array.from(
        root.querySelectorAll<TestOwnerProbe>('test-owner-probe'),
      );
      return {
        hasShadow: host.shadowRoot !== null,
        mediaClasses: media.map(element => element.className),
        mediaOwned: media.every(element => element.ownerDocument === document),
        probes: probes.map(probe => ({
          connectedCalls: probe.connectedCalls,
          connectedOwner: probe.connectedOwnerDocument === document,
          constructedConnected: probe.constructedConnected,
          constructedOwner: probe.constructedOwnerDocument === document,
          kind: probe.dataset.kind,
        })),
      };
    }, tag);

    expect(result).toEqual({
      hasShadow: tag === 'test-owner-shadow',
      mediaClasses: ['direct', 'conditional', 'repeat'],
      mediaOwned: true,
      probes: [
        {
          connectedCalls: 1,
          connectedOwner: true,
          constructedConnected: false,
          constructedOwner: true,
          kind: 'direct',
        },
        {
          connectedCalls: 1,
          connectedOwner: true,
          constructedConnected: false,
          constructedOwner: true,
          kind: 'conditional',
        },
        {
          connectedCalls: 1,
          connectedOwner: true,
          constructedConnected: false,
          constructedOwner: true,
          kind: 'repeat',
        },
      ],
    });
  });
}

declare global {
  interface TestOwnerProbe extends HTMLElement {
    readonly constructedConnected: boolean;
    readonly constructedOwnerDocument: Document;
    readonly connectedCalls: number;
    readonly connectedOwnerDocument: Document | null;
  }
}
