// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '../../../src/runtime.js';
import { registerComponentStyles } from '../../../src/element/style-catalog.js';
import type { TemplateInstance } from '../../../src/element/types.js';
import type { TemplateBlockMeta } from '../../../src/template-types.js';
import '../../../src/styles-entry.js';
import {
  WebUIElement,
  attr,
  observable,
  registerTemplateData,
  type TemplateMeta,
} from '../../../src/index.js';

startRuntime();

const templates: Record<string, TemplateMeta> = {
  'test-runtime-life': {
    v: 4,
    h: '<span></span>',
    l: [3, 1],
    tx: [
      [[1, 0], [['label']]],
    ],
    tr: ['label'],
    ta: ['label'],
    th: 1,
  },
  'test-runtime-effects': {
    v: 4,
    h: '<span></span>',
    l: [3, 1],
    th: 1,
  },
  'test-runtime-throw': {
    v: 4,
    h: '<span></span>',
    l: [3, 1],
    th: 1,
  },
  'test-runtime-immediate': {
    v: 4,
    h: '<span></span>',
    l: [3, 1],
  },
  'test-runtime-static-shadow': { v: 3, h: '<span>Ready</span>', l: [3, 1], sd: 1 },
  'test-runtime-custom-finalizer': { v: 3, h: '<span>Ready</span>', l: [3, 1], sd: 1 },
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

export class TestRuntimeStaticShadow extends WebUIElement {
  @attr label = 'initial';
  styleDuringReflection = '';
  hydratedCalls = 0;
  reflectedContent: string | null | undefined;

  override attributeChangedCallback(name: string, before: string | null, after: string | null): void {
    super.attributeChangedCallback(name, before, after);
    if (name === 'label' && this.hydratedCalls === 0 && this.shadowRoot) {
      this.styleDuringReflection = this.shadowRoot.querySelector('style')?.textContent ?? '';
      this.reflectedContent = this.shadowRoot.querySelector('span')?.textContent;
    }
  }

  protected override hydratedCallback(): void {
    this.hydratedCalls++;
    this.shadowRoot!.querySelector('span')!.setAttribute('data-hydrated', '');
  }
}

class TestRuntimeCustomFinalizer extends WebUIElement {
  protected override $finalize(
    instance: TemplateInstance,
    meta: TemplateBlockMeta,
    elements: readonly (Node | undefined)[],
  ): void {
    super.$finalize(instance, meta, elements);
    (elements[1] as Element).setAttribute('data-finalized', '');
  }
}

function registerTemplates(): void {
  registerTemplateData(templates);
}

const streaming = !!document.querySelector('meta[name="webui-streaming"][content="1"]');
const routerLate = !!document.querySelector('meta[name="webui-delivery-late"][content="1"]');
// Ordinary pages publish metadata synchronously; streaming and Router-late
// pages hold definitions until that same metadata registration arrives.
TestRuntimeImmediate.define('test-runtime-immediate');
if (!streaming && !routerLate) {
  registerTemplates();
}

TestRuntimeLife.define('test-runtime-life');
TestRuntimeEffects.define('test-runtime-effects');
TestRuntimeThrow.define('test-runtime-throw');

if (!streaming && !routerLate) {
  registerComponentStyles({
    version: 1,
    strategy: 'style',
    resources: { 'test-runtime-static-shadow': { kind: 'style', css: 'span{color:red}' } },
    closures: { 'test-runtime-static-shadow': ['test-runtime-static-shadow'] },
  });
  TestRuntimeStaticShadow.define('test-runtime-static-shadow');
  TestRuntimeCustomFinalizer.define('test-runtime-custom-finalizer');
}

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
