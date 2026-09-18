// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { describe, test } from 'node:test';
import assert from 'node:assert/strict';

import { getTemplate, releaseSSRBootstrapState } from './template.js';
import { clearFragmentInputSources, fragmentInput } from './fragment-inputs.js';

interface FakeNode {
  nodeType: number;
  data?: string;
  firstChild: FakeNode | null;
  nextSibling: FakeNode | null;
  parentNode: FakeNode | null;
  shadowRoot?: FakeNode | null;
  host?: FakeNode | null;
}

function fakeComment(data: string): FakeNode {
  return { nodeType: 8, data, firstChild: null, nextSibling: null, parentNode: null };
}

/** Link `children` into `parent` in document order. */
function fakeParent(nodeType: number, children: FakeNode[]): FakeNode {
  const parent: FakeNode = {
    nodeType,
    firstChild: children[0] ?? null,
    nextSibling: null,
    parentNode: null,
  };
  for (let i = 0; i < children.length; i++) {
    children[i].parentNode = parent;
    children[i].nextSibling = children[i + 1] ?? null;
  }
  return parent;
}

// `loadWebUIDataBlock` latches after its first run and the latch is module
// state, so this file owns one load of its own; `node --test` gives every test
// file its own process.
describe('SSR data block captured fragment inputs', () => {
  test('decodes captured inputs and keeps them off the runtime global', () => {
    const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    const previousDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');

    try {
      const marker = fakeComment('wf:1');
      const consumed = fakeComment('');
      const shadow = fakeParent(11, [marker, consumed]);
      const host: FakeNode = {
        nodeType: 1,
        firstChild: null,
        nextSibling: null,
        parentNode: null,
        shadowRoot: shadow,
      };
      shadow.host = host;
      const documentBody = fakeParent(1, [host]);

      Object.defineProperty(globalThis, 'window', {
        value: {},
        configurable: true,
        writable: true,
      });
      Object.defineProperty(globalThis, 'document', {
        value: {
          body: documentBody,
          getElementById(id: string) {
            if (id !== 'webui-data') return null;
            return {
              textContent: '{"state":{"source":{"label":"NEW"}},'
                + '"fragmentSources":[[0,0,{"label":"OLD"}],[1,1,0,"label"]],'
                + '"fragmentSourceRefs":[1],'
                + '"templates":{"greeting":{"h":"<p></p>"}}}',
              remove() {},
            };
          },
        },
        configurable: true,
        writable: true,
      });

      assert.equal(getTemplate('greeting')?.h, '<p></p>');
      // Provenance is decoded once, before anything in the document can
      // hydrate, and every identifier resolves to the captured value.
      assert.deepEqual(fragmentInput({} as Element, 0), { label: 'OLD' });
      assert.equal(fragmentInput({} as Element, 1), 'OLD');
      // Page state stays exactly what the server sent; the capture keys are
      // transport, and publishing them would make them observable app state.
      assert.deepEqual(window.__webui!.state, { source: { label: 'NEW' } });
      assert.equal(
        Object.prototype.hasOwnProperty.call(window.__webui!, 'fragmentSources'),
        false,
      );
      assert.equal(
        Object.prototype.hasOwnProperty.call(window.__webui!, 'fragmentSourceRefs'),
        false,
      );

      // A buffered response has no terminal record, so startup hydration is
      // where its table must close. Everything that hydrated already adopted
      // its input; the markers of hosts that are still dormant take their value
      // with them, so retention follows those nodes instead of the page.
      releaseSSRBootstrapState();
      assert.equal(window.__webui!.state, undefined);
      assert.throws(
        () => fragmentInput({} as Element, 0),
        /unknown source 0 after the response closed/,
        'the decoded table does not outlive startup hydration',
      );
      assert.equal(
        fragmentInput(host as unknown as Element, 1, marker as unknown as Comment),
        'OLD',
        'a dormant shadow-tree marker still resolves the input it was streamed with',
      );
      assert.throws(
        () => fragmentInput(host as unknown as Element, 1, consumed as unknown as Comment),
        /unknown source 1 after the response closed/,
        'a marker that already hydrated retains nothing, and skew is reported rather than guessed',
      );
    } finally {
      clearFragmentInputSources(true);
      if (previousWindow) {
        Object.defineProperty(globalThis, 'window', previousWindow);
      } else {
        Reflect.deleteProperty(globalThis, 'window');
      }
      if (previousDocument) {
        Object.defineProperty(globalThis, 'document', previousDocument);
      } else {
        Reflect.deleteProperty(globalThis, 'document');
      }
    }
  });
});
