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

  test(`${tag} mounts and updates client-created content in an iframe document`, async ({ page }) => {
    await page.evaluate(() => {
      const iframe = document.createElement('iframe');
      iframe.id = 'owner-document-frame';
      document.body.appendChild(iframe);
    });
    const frame = page.frameLocator('#owner-document-frame');
    await frame.locator('body').evaluate(() => {
      // Native iframe constructors isolate DOM ownership from WebUI registry support.
      class IframeOwnerProbe extends HTMLElement {
        readonly constructedConnected = this.isConnected;
        readonly constructedOwnerDocument = this.ownerDocument;
        connectedCalls = 0;
        connectedOwnerDocument: Document | null = null;

        connectedCallback(): void {
          this.connectedCalls++;
          this.connectedOwnerDocument = this.ownerDocument;
        }
      }
      customElements.define('test-owner-probe', IframeOwnerProbe);
    });

    const result = await page.evaluate((tag) => {
      const iframe = document.querySelector<HTMLIFrameElement>('#owner-document-frame');
      const target = iframe?.contentDocument;
      if (!target) throw new Error('fixture iframe has no document');
      const host = document.createElement(tag) as TestOwnerHost;
      target.adoptNode(host);

      const stagingDocuments: boolean[] = [];
      const globalCreations: string[] = [];
      const upgrade = customElements.upgrade;
      const createTextNode = document.createTextNode;
      const createComment = document.createComment;
      const createDocumentFragment = document.createDocumentFragment;

      // Observe staging and factory calls before DOM insertion can silently adopt nodes.
      customElements.upgrade = (root) => {
        stagingDocuments.push(root.ownerDocument === target);
        upgrade.call(customElements, root);
      };
      document.createTextNode = (data) => {
        globalCreations.push('text');
        return createTextNode.call(document, data);
      };
      document.createComment = (data) => {
        globalCreations.push('comment');
        return createComment.call(document, data);
      };
      document.createDocumentFragment = () => {
        globalCreations.push('fragment');
        return createDocumentFragment.call(document);
      };

      const snapshot = () => {
        const root = host.shadowRoot ?? host;
        const walker = target.createTreeWalker(root, NodeFilter.SHOW_ALL);
        let owned = root.ownerDocument === target;
        let hasText = false;
        let hasAnchor = false;
        let node = walker.nextNode();
        while (node) {
          owned = owned && node.ownerDocument === target;
          hasText = hasText || node.nodeType === Node.TEXT_NODE;
          hasAnchor = hasAnchor || node.nodeType === Node.COMMENT_NODE;
          node = walker.nextNode();
        }
        return {
          ready: host.$ready,
          hostOwned: host.ownerDocument === target,
          hasShadow: host.shadowRoot !== null,
          owned,
          hasText,
          hasAnchor,
          labels: Array.from(root.querySelectorAll('.label'), element => element.textContent),
          itemIds: Array.from(root.querySelectorAll('section'), element => element.dataset.item),
          itemText: Array.from(root.querySelectorAll('.item-id'), element => element.textContent),
          mediaClasses: Array.from(root.querySelectorAll('video'), element => element.className),
          probes: Array.from(root.querySelectorAll<TestOwnerProbe>('test-owner-probe'), probe => ({
            connectedCalls: probe.connectedCalls,
            connectedOwner: probe.connectedOwnerDocument === target,
            constructedConnected: probe.constructedConnected,
            constructedOwner: probe.constructedOwnerDocument === target,
            kind: probe.dataset.kind,
          })),
        };
      };

      try {
        target.body.appendChild(host);
        const initial = snapshot();
        host.label = 'updated';
        host.showConditional = false;
        host.items = [];
        host.$flushUpdates();
        const cleared = snapshot();
        host.showConditional = true;
        host.items = [{ id: 'second' }, { id: 'third' }];
        host.$flushUpdates();
        return {
          initial,
          cleared,
          rebuilt: snapshot(),
          globalCreations,
          sawStagingRoots: stagingDocuments.length > 0,
          stagingOwned: stagingDocuments.every(Boolean),
        };
      } finally {
        customElements.upgrade = upgrade;
        document.createTextNode = createTextNode;
        document.createComment = createComment;
        document.createDocumentFragment = createDocumentFragment;
      }
    }, tag);

    const common = {
      ready: true,
      hostOwned: true,
      hasShadow: tag === 'test-owner-shadow',
      owned: true,
      hasText: true,
      hasAnchor: true,
    };
    const expectedProbe = (kind: string) => ({
      connectedCalls: 1,
      connectedOwner: true,
      constructedConnected: false,
      constructedOwner: true,
      kind,
    });
    expect(result.initial).toEqual({
      ...common,
      labels: ['initial', 'initial'],
      itemIds: ['first'],
      itemText: ['first'],
      mediaClasses: ['direct', 'conditional', 'repeat'],
      probes: ['direct', 'conditional', 'repeat'].map(expectedProbe),
    });
    expect(result.cleared).toEqual({
      ...common,
      labels: ['updated'],
      itemIds: [],
      itemText: [],
      mediaClasses: ['direct'],
      probes: [expectedProbe('direct')],
    });
    expect(result.rebuilt).toEqual({
      ...common,
      labels: ['updated', 'updated'],
      itemIds: ['second', 'third'],
      itemText: ['second', 'third'],
      mediaClasses: ['direct', 'conditional', 'repeat', 'repeat'],
      probes: ['direct', 'conditional', 'repeat', 'repeat'].map(expectedProbe),
    });
    expect(result.globalCreations).toEqual([]);
    expect(result.sawStagingRoots).toBe(true);
    expect(result.stagingOwned).toBe(true);
    await expect(frame.locator(`${tag} .label`)).toHaveText(['updated', 'updated']);
    await expect(frame.locator(`${tag} .item-id`)).toHaveText(['second', 'third']);
    expect(await page.pageErrors()).toEqual([]);
  });
}

declare global {
  interface TestOwnerHost extends HTMLElement {
    label: string;
    showConditional: boolean;
    items: { id: string }[];
    readonly $ready: boolean;
    $flushUpdates(): void;
  }

  interface TestOwnerProbe extends HTMLElement {
    readonly constructedConnected: boolean;
    readonly constructedOwnerDocument: Document;
    readonly connectedCalls: number;
    readonly connectedOwnerDocument: Document | null;
  }
}
