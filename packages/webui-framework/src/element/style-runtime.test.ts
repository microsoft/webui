// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import type { TemplateMeta } from '../template-types.js';
import { registerComponentStyles } from './style-catalog.js';
import {
  componentStyleRuntime,
  installComponentStyleRuntime,
  requireComponentStyleRuntime,
  type ComponentStyleRuntime,
} from './style-runtime.js';

function ownerDocument(): Document {
  return Object.create(null) as Document;
}

test('unstyled core components do not require a stylesheet runtime', () => {
  const owner = ownerDocument();
  const meta: TemplateMeta = { v: 3, h: '<p>Plain content</p>', b: [{ h: '<span></span>' }] };
  assert.equal(requireComponentStyleRuntime('plain-card', meta, owner), undefined);
  assert.equal(requireComponentStyleRuntime('plain-card', meta, owner), undefined);
  assert.equal(componentStyleRuntime, undefined);
});

test('registered CSS fails explicitly when only the core entry was loaded', () => {
  const owner = ownerDocument();
  registerComponentStyles({
    version: 1,
    strategy: 'style',
    resources: {
      'styled-card': { kind: 'style', css: 'p { color: red; }' },
    },
    closures: { 'styled-card': ['styled-card'] },
  }, owner);

  assert.throws(
    () => requireComponentStyleRuntime('styled-card', { v: 3, h: '<p></p>' }, owner),
    /Styles for <styled-card> require @microsoft\/webui-framework\/styles\.js/,
  );
});

test('template-owned links also require the stylesheet runtime', () => {
  for (const meta of [
    { v: 3, h: '<link rel="stylesheet" href="card.css">' },
    { v: 3, h: '<section></section>', b: [{ h: '<LINK rel="stylesheet" href="card.css">' }] },
  ] satisfies TemplateMeta[]) {
    assert.throws(
      () => requireComponentStyleRuntime('linked-card', meta, ownerDocument()),
      /Import that entry before defining styled components/,
    );
  }
});

test('one installed runtime is shared without allocating per-component adapters', () => {
  const runtime: ComponentStyleRuntime = {
    claimSsr: () => {},
    install: () => undefined,
    prepareTemplate: () => undefined,
    hasTemplateLinks: () => false,
    installTemplateLinks: () => false,
    cancelTemplateMount: () => false,
  };
  installComponentStyleRuntime(runtime);
  installComponentStyleRuntime(runtime);
  assert.equal(componentStyleRuntime, runtime);
  assert.equal(
    requireComponentStyleRuntime('any-card', { v: 3, h: '<p></p>' }, ownerDocument()),
    runtime,
  );
  assert.throws(
    () => installComponentStyleRuntime({ ...runtime }),
    /A component style runtime is already installed/,
  );
});
