// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { writeFileSync } from 'node:fs';

/** Verify useful work in a separate context, never in the measured heap. */
export async function verifyTriggerFixture(browser, url, target, legacy, packName) {
  const context = await browser.newContext();
  try {
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(String(error)));
    await page.goto(url);
    await page.waitForFunction(() => window.__bench?.hydrated);
    const proof = await page.evaluate(async ({ target, legacy }) => {
      const app = document.querySelector('bench-app');
      const finish = async element => {
        if (legacy) element.$flushUpdates();
        else await element.$whenRendered();
      };
      if (!target) {
        app.title = 'Updated workspace';
        await finish(app);
        return {
          definitions: app instanceof customElements.get('bench-app'),
          startupText: app.shadowRoot.querySelector('h1').textContent === 'Updated workspace',
          legacy,
        };
      }
      const owner = app.shadowRoot.querySelector(target);
      const result = {
        definitions: [app, owner, app.shadowRoot.querySelector('bench-badge')]
          .every(element => element instanceof customElements.get(element.localName)),
        rootRef: owner.dialog === owner.shadowRoot.querySelector('dialog'),
        ownerStyle: getComputedStyle(owner.dialog).borderRadius === '12px',
      };
      app.title = 'Updated workspace';
      await finish(app);
      result.startupText = app.shadowRoot.querySelector('h1').textContent === 'Updated workspace';
      owner.shadowRoot.querySelector('.trigger').click();
      await pageResult();
      result.event = owner.dialog.open;
      result.bodyRef = owner.introNode === owner.dialog.querySelector('.intro');
      result.repeat = owner.dialog.querySelectorAll('.items li').length === 5;
      const selects = [...owner.dialog.querySelectorAll('select')];
      result.independentRepeats = selects.length === 6
        && selects[0].options[0] !== selects[5].options[0];
      owner.intro = 'Updated introduction';
      owner.text = 'Updated text';
      owner.enabled = false;
      owner.badge = 'Updated badge';
      owner.items = [{ name: 'Updated row', detail: 'Updated detail' }];
      owner.options = [{ value: 'updated', label: 'Updated option' }];
      await finish(owner);
      result.text = owner.dialog.querySelector('.intro').textContent === 'Updated introduction';
      result.attribute = owner.dialog.querySelector('input:not([type=checkbox])').value === 'Updated text';
      result.boolean = !owner.dialog.querySelector('input[type=checkbox]').checked;
      result.childState = owner.dialog.querySelector('bench-badge').label === 'Updated badge';
      result.repeatUpdate = owner.dialog.querySelectorAll('.items li').length === 1
        && owner.dialog.querySelector('.items li strong').textContent === 'Updated row';
      result.sharedRepeatUpdates = selects.every(select => select.options.length === 1
        && select.options[0].value === 'updated' && select.options[0].textContent === 'Updated option');
      const oldRef = owner.introNode;
      owner.dialog.querySelector('.close').click();
      await finish(owner);
      result.closeEvent = !owner.dialog.open && !owner.ready;
      result.removal = !owner.dialog.querySelector('.intro');
      // Older main did not release body refs. Record that contract difference;
      // do not pretend its lower cost provides the same cleanup guarantee.
      result.releasesRef = owner.introNode === undefined;
      result.legacy = legacy;
      result.detachedRef = oldRef && !oldRef.isConnected;
      const created = document.createElement(target);
      app.shadowRoot.append(created);
      await finish(created);
      result.createdRef = created.dialog === created.shadowRoot.querySelector('dialog');
      created.ready = true;
      await finish(created);
      result.createdBody = created.introNode === created.dialog.querySelector('.intro')
        && created.dialog.querySelectorAll('.items li').length === 5;
      result.createdStyle = getComputedStyle(created.dialog).borderRadius === '12px';
      created.remove();
      return result;
      async function pageResult() {
        while (!window.__bench.result && !window.__bench.error) await new Promise(resolve => setTimeout(resolve, 0));
        if (window.__bench.error) throw new Error(window.__bench.error);
      }
    }, { target, legacy });
    for (const [key, value] of Object.entries(proof)) {
      if (key !== 'legacy' && !(legacy && key === 'releasesRef')) {
        assert.equal(value, true, `Equivalent useful work failed: ${key}`);
      }
    }
    assert.deepEqual(errors, [], 'The comparison must not hide runtime errors');
    if (packName) {
      proof.standaloneFetch = await page.evaluate(async url => {
        const start = performance.now();
        const response = await fetch(url, { priority: 'high' });
        const headers = performance.now();
        await response.json();
        const end = performance.now();
        const resource = performance.getEntriesByName(url).at(-1);
        return { headersMs: headers - start, decodeMs: end - headers, responseMs: resource.responseEnd - start };
      }, `${url}_webui/packs/${packName}?standalone-proof`);
      await page.evaluate(url => {
        const button = document.createElement('button');
        button.id = 'fetch-only-proof';
        button.textContent = 'Fetch-only control';
        button.style.cssText = 'position:fixed;top:0;left:0;z-index:100000';
        button.onclick = async () => {
          try {
            const start = performance.now();
            const response = await fetch(url, { priority: 'high' });
            const headers = performance.now();
            await response.json();
            const resource = performance.getEntriesByName(url).at(-1);
            window.__fetchOnlyProof = {
              headersMs: headers - start, decodeMs: performance.now() - headers,
              responseMs: resource.responseEnd - start,
            };
          } catch (error) {
            window.__fetchOnlyProof = { error: String(error) };
          }
        };
        document.body.append(button);
      }, `${url}_webui/packs/${packName}?input-proof`);
      await page.locator('#fetch-only-proof').click();
      await page.waitForFunction(() => window.__fetchOnlyProof);
      proof.inputFetch = await page.evaluate(() => window.__fetchOnlyProof);
      assert.equal(proof.inputFetch.error, undefined);
    }
    return proof;
  } finally {
    await context.close();
  }
}

/** Snapshots are an explicitly separate attribution run, not timing samples. */
export async function snapshot(cdp, path) {
  const chunks = [];
  const chunk = event => chunks.push(event.chunk);
  cdp.on('HeapProfiler.addHeapSnapshotChunk', chunk);
  try {
    await cdp.send('HeapProfiler.takeHeapSnapshot', { reportProgress: false });
    writeFileSync(path, chunks.join(''));
  } finally {
    cdp.off('HeapProfiler.addHeapSnapshotChunk', chunk);
  }
}

/** Observe pack decode, registry notification and template construction without changing their order. */
export function installTriggerProbe() {
  const spans = [];
  const record = (kind, start) => spans.push({ kind, start, end: performance.now() });
  const fetch = window.fetch;
  window.fetch = function (...args) {
    const start = performance.now();
    return fetch.apply(this, args).then(response => {
      record('fetchHeaders', start);
      return response;
    });
  };
  const json = Response.prototype.json;
  Response.prototype.json = function (...args) {
    const start = performance.now();
    return json.apply(this, args).then(value => {
      record('json', start);
      return value;
    });
  };
  const dispatch = window.dispatchEvent;
  window.dispatchEvent = function (event) {
    const start = performance.now();
    try { return dispatch.call(this, event); }
    finally {
      if (event.type === 'webui:templates-registered') record('registration', start);
    }
  };
  const clone = Document.prototype.importNode;
  Document.prototype.importNode = function (...args) {
    const start = performance.now();
    try { return clone.apply(this, args); }
    finally { record('clone', start); }
  };
  const html = Object.getOwnPropertyDescriptor(Element.prototype, 'innerHTML');
  Object.defineProperty(Element.prototype, 'innerHTML', {
    ...html,
    set(value) {
      const start = performance.now();
      try { html.set.call(this, value); }
      finally { record('html', start); }
    },
  });
  const raf = window.requestAnimationFrame;
  window.requestAnimationFrame = callback => {
    const start = performance.now();
    return raf(time => { record('frameWait', start); callback(time); });
  };
  window.__triggerSpans = spans;
}
