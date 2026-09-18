// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';
import type { FlatRootProbe } from './element.js';

test.beforeEach(async ({ page }) => {
  await page.goto('/flat-root-ownership/fixture.html');
  await page.waitForFunction(() => document.querySelector<FlatRootProbe>('test-flat-one')?.fixtureRoot !== undefined);
});

for (const mode of ['SSR', 'CSR']) {
  for (const [kind, count] of [['zero', 0], ['one', 1], ['two', 2], ['many', 6], ['empty-text', 1]] as const) {
    test(`${mode} ${kind}: omit root storage before wiring, preserve update/move/reconnect/teardown`, async ({ page }) => {
      const result = await page.evaluate(async ({ mode, kind, count }) => {
        const name = `test-flat-${kind}`;
        const host = mode === 'CSR'
          ? document.createElement(name) as FlatRootProbe
          : document.querySelector<FlatRootProbe>(name);
        if (!host) throw new Error('missing fixture host');
        if (mode === 'CSR') document.body.append(host);
        const root = host.shadowRoot, firstInstance = host.fixtureRoot;
        if (!root || !firstInstance) throw new Error('mount did not complete synchronously');
        const members = Array.from(root.childNodes);
        const omitted = host.omittedDuringWiring && Object.isFrozen(firstInstance.nodes) && firstInstance.nodes.length === 0;
        const cardinality = members.length === count;
        const span = root.querySelector('span');
        const text = kind === 'empty-text' ? root.firstChild : span?.firstChild;
        host.label = 'changed';
        await Promise.resolve();
        const updated = kind === 'zero' || (text?.textContent === 'changed' && (!span || span.title === 'changed'));
        host.dispatchEvent(new Event('click'));
        const wiredOnce = host.finalizations === 1 && host.clicks === 1;
        const foreign = document.createComment('external');
        root.insertBefore(foreign, root.lastChild);
        const parking = document.createElement('section');
        document.body.append(parking);
        parking.append(host);
        await Promise.resolve();
        const nativeMove = host.fixtureRoot === firstInstance && host.hydrations === 1;
        for (let i = 0; i < members.length; i++) {
          if (members[i].parentNode !== root) throw new Error('host move replaced a top-level node');
        }
        host.remove();
        await Promise.resolve();
        const detachedInstance = host.fixtureRoot === undefined;
        const cleaned = firstInstance.texts.length === 0 && (firstInstance.cleanups?.length ?? 0) === 0;
        host.dispatchEvent(new Event('click'));
        const removedListener = host.clicks === 1;
        host.label = 'reconnected';
        document.body.append(host);
        const nextInstance = host.fixtureRoot;
        const reconnected = nextInstance !== undefined && nextInstance !== firstInstance &&
          nextInstance.nodes === firstInstance.nodes && host.omittedDuringWiring && host.hydrations === 1;
        host.dispatchEvent(new Event('click'));
        const reboundOnce = host.clicks === 2;
        const retained = members.every(node => node.parentNode === root) && foreign.parentNode === root;
        const reboundText = kind === 'zero' || text?.textContent === 'reconnected';
        host.$destroy();
        const destroyRetainsDOM = foreign.parentNode === root && members.every(node => node.parentNode === root);
        return { omitted, cardinality, updated, wiredOnce, nativeMove, detachedInstance, cleaned,
          removedListener, reconnected, reboundOnce, retained, reboundText, destroyRetainsDOM };
      }, { mode, kind, count });
      expect(Object.values(result).every(Boolean), JSON.stringify(result)).toBe(true);
    });
  }
}

test('external removal/moves retain direct binding semantics without discovering new ownership', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const host = document.querySelector<FlatRootProbe>('test-flat-many');
    if (!host?.shadowRoot || !host.fixtureRoot) throw new Error('missing fixture');
    const root = host.shadowRoot, instance = host.fixtureRoot;
    const span = root.querySelector('span'), bold = root.querySelector('b');
    if (!span || !bold) throw new Error('missing managed nodes');
    const external = document.createElement('u'), parking = document.createElement('section');
    root.insertBefore(external, bold);
    document.body.append(parking);
    parking.append(span);
    bold.remove();
    host.label = 'moved binding';
    await Promise.resolve();
    const direct = span.textContent === 'moved binding' && span.title === 'moved binding';
    const untouched = span.parentNode === parking && external.parentNode === root && bold.parentNode === null;
    host.$destroy();
    return {
      direct, untouched, omitted: instance.nodes.length === 0 && Object.isFrozen(instance.nodes),
      retainedOutside: span.parentNode === parking && external.parentNode === root,
      releasedBindings: instance.texts.length === 0 && instance.attrs.length === 0,
    };
  });
  expect(Object.values(result).every(Boolean), JSON.stringify(result)).toBe(true);
});

test('flat, conditional, repeat, raw and fragment roots coexist without sharing mutable storage', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const outcomes: boolean[] = [];
    for (const mode of ['SSR', 'CSR']) {
      for (const kind of ['condition', 'repeat', 'raw', 'fragment']) {
        const name = `test-owned-${kind}`;
        const host = mode === 'SSR' ? document.querySelector<FlatRootProbe>(name)
          : document.createElement(name) as FlatRootProbe;
        if (!host) throw new Error('missing structural host');
        if (mode === 'CSR') document.body.append(host);
        const instance = host.fixtureRoot;
        if (!instance || !host.shadowRoot) throw new Error('missing structural instance');
        outcomes.push(!host.omittedDuringWiring && !Object.isFrozen(instance.nodes));
        host.label = 'updated';
        host.items = ['one', 'two'];
        host.html = '<b>updated raw</b>';
        await Promise.resolve();
        outcomes.push(kind === 'raw' ? host.shadowRoot.textContent === 'updated raw'
          : kind === 'repeat' ? host.shadowRoot.textContent === 'onetwo' : host.shadowRoot.textContent === 'updated');
        if (kind === 'condition') {
          host.show = false;
          await Promise.resolve();
          host.show = true;
          await Promise.resolve();
          outcomes.push(host.shadowRoot.querySelectorAll('span').length === 1);
        }
        const flat = document.querySelector<FlatRootProbe>('test-flat-one')?.fixtureRoot;
        outcomes.push(instance.nodes !== flat?.nodes);
      }
    }
    return outcomes;
  });
  expect(result.every(Boolean)).toBe(true);
});

test('new registration does not retrofit topology or ownership into an already mounted root', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const host = document.querySelector<FlatRootProbe>('test-flat-one');
    if (!host?.shadowRoot || !host.fixtureRoot) throw new Error('missing fixture');
    const root = host.shadowRoot, original = root.firstChild, instance = host.fixtureRoot;
    host.replaceRegistration();
    host.label = 'still original';
    await Promise.resolve();
    const retained = root.firstChild === original && root.textContent === 'still original' && host.fixtureRoot === instance;
    host.remove();
    await Promise.resolve();
    document.body.append(host);
    return { retained, reconnected: root.firstChild === original && root.querySelector('aside') === null,
      omitted: host.omittedDuringWiring && host.fixtureRoot?.nodes === instance.nodes };
  });
  expect(result).toEqual({ retained: true, reconnected: true, omitted: true });
});

for (const mode of ['SSR', 'CSR']) {
  test(`${mode}: a structural parent removes/readds a flat component through host ownership`, async ({ page }) => {
    const result = await page.evaluate(async mode => {
      const parent = mode === 'SSR' ? document.querySelector<FlatRootProbe>('test-owned-host')
        : document.createElement('test-owned-host') as FlatRootProbe;
      if (!parent) throw new Error('missing structural parent');
      if (mode === 'CSR') document.body.append(parent);
      await Promise.resolve();
      const child = parent.shadowRoot?.querySelector<FlatRootProbe>('test-flat-one');
      const instance = child?.fixtureRoot, root = child?.shadowRoot;
      if (!child || !instance || !root) throw new Error('missing nested flat root');
      const parentOwnsHost = parent.fixtureRoot?.conds[0].instance?.nodes.includes(child) === true;
      const flat = child.omittedDuringWiring && Object.isFrozen(instance.nodes);
      const content = root.firstChild;
      parent.show = false;
      await Promise.resolve();
      await Promise.resolve();
      child.dispatchEvent(new Event('click'));
      const removed = !child.isConnected && child.fixtureRoot === undefined &&
        child.clicks === 0 && root.firstChild === content && instance.texts.length === 0;
      parent.show = true;
      await Promise.resolve();
      await Promise.resolve();
      const replacement = parent.shadowRoot?.querySelector<FlatRootProbe>('test-flat-one');
      if (!replacement?.shadowRoot || !replacement.fixtureRoot) throw new Error('missing replacement child');
      replacement.dispatchEvent(new Event('click'));
      return { parentOwnsHost, flat, removed,
        readded: replacement !== child && replacement.omittedDuringWiring &&
          replacement.fixtureRoot.nodes === instance.nodes && replacement.clicks === 1 };
    }, mode);
    expect(result).toEqual({ parentOwnsHost: true, flat: true, removed: true, readded: true });
  });
}
