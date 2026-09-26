// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  WebUIElement,
  attr,
  observable,
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
  'test-runtime-effects': {
    h: '<span></span>',
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
  @observable count = 0;
  hydratedCalls = 0;
  propertyCalls: { oldValue: unknown; value: number; connected: boolean }[] = [];
  attributeChanges: string[] = [];

  countChanged(oldValue: unknown, value: number): void {
    this.propertyCalls.push({ oldValue, value, connected: this.isConnected });
  }

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

export class TestRuntimeEffects extends WebUIElement {
  @observable a = 0;
  @observable b = 0;
  @attr label = 'default';
  calls: (
    | { name: 'a' | 'b'; oldValue: number | undefined; value: number; connected: boolean }
    | { name: 'hydrated'; connected: boolean }
  )[] = [];
  onAChange?: (oldValue: number | undefined, value: number) => void;

  aChanged(oldValue: number | undefined, value: number): void {
    this.calls.push({ name: 'a', oldValue, value, connected: this.isConnected });
    this.onAChange?.(oldValue, value);
  }

  bChanged(oldValue: number | undefined, value: number): void {
    this.calls.push({ name: 'b', oldValue, value, connected: this.isConnected });
  }

  protected override hydratedCallback(): void {
    this.calls.push({ name: 'hydrated', connected: this.isConnected });
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
TestRuntimeEffects.define('test-runtime-effects');
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
