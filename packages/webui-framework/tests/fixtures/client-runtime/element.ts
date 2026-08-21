// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  WebUIElement,
  registerTemplateData,
  type TemplateMeta,
} from '../../../src/index.js';

const templates: Record<string, TemplateMeta> = {
  'test-runtime-life': {
    h: '<span></span>',
    tx: [
      [[1, 0], [['label']]],
    ],
    tr: ['label'],
    ta: ['label'],
    th: 1,
  },
  'test-runtime-throw': {
    h: '<span></span>',
    th: 1,
  },
  'test-runtime-immediate': {
    h: '<span></span>',
  },
};

export class TestRuntimeLife extends WebUIElement {
  hydratedCalls = 0;
  attributeChanges: string[] = [];

  protected override hydratedCallback(): void {
    this.hydratedCalls++;
  }

  override attributeChangedCallback(
    name: string,
    oldValue: string | null,
    newValue: string | null,
  ): void {
    super.attributeChangedCallback(name, oldValue, newValue);
    this.attributeChanges.push(name);
  }
}

export class TestRuntimeThrow extends WebUIElement {
  hydratedCalls = 0;

  protected override hydratedCallback(): void {
    this.hydratedCalls++;
    throw new Error('expected hydratedCallback failure');
  }
}

export class TestRuntimeImmediate extends WebUIElement {
}

function registerTemplates(): void {
  registerTemplateData(templates);
}

const streaming = !!document.querySelector('meta[name="webui-streaming"][content="1"]');
const routerLate = !!document.querySelector('meta[name="webui-router-late"][content="1"]');
// Ordinary pages publish metadata synchronously; streaming and Router-late
// pages hold definitions until that same metadata registration arrives.
TestRuntimeImmediate.define('test-runtime-immediate');
if (!streaming && !routerLate) {
  registerTemplates();
}

TestRuntimeLife.define('test-runtime-life');
TestRuntimeThrow.define('test-runtime-throw');

window.TestRuntimeLife = TestRuntimeLife;
window.TestRuntimeThrow = TestRuntimeThrow;
window.TestRuntimeImmediate = TestRuntimeImmediate;
window.registerClientRuntimeTemplates = registerTemplates;

declare global {
  interface Window {
    TestRuntimeLife: typeof TestRuntimeLife;
    TestRuntimeThrow: typeof TestRuntimeThrow;
    TestRuntimeImmediate: typeof TestRuntimeImmediate;
    registerClientRuntimeTemplates(): void;
  }
}
