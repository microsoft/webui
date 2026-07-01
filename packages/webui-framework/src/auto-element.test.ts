// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';
import type { TemplateMeta } from './template.js';

const registry = new Map<string, CustomElementConstructor>();

Object.defineProperty(globalThis, 'HTMLElement', {
  value: class HTMLElement {
    isConnected = false;

    get title(): string {
      return 'native title';
    }

    hasAttribute(_name: string): boolean {
      return false;
    }
  },
  configurable: true,
});

Object.defineProperty(globalThis, 'customElements', {
  value: {
    get(name: string): CustomElementConstructor | undefined {
      return registry.get(name);
    },
    define(name: string, ctor: CustomElementConstructor): void {
      registry.set(name, ctor);
    },
  },
  configurable: true,
});

Object.defineProperty(globalThis, 'document', {
  value: {
    readyState: 'complete',
    getElementById() {
      return null;
    },
  },
  configurable: true,
});

Object.defineProperty(globalThis, 'window', {
  value: {
    __webui: { templates: {} },
    addEventListener() {},
  },
  configurable: true,
});

const {
  installAutoElementRuntime,
} = await import('./auto-element.js');

type ObservedElementConstructor = CustomElementConstructor & {
  readonly observedAttributes: readonly string[];
};

function textTemplate(path: string, autoElement = true): TemplateMeta {
  return {
    h: '<p></p>',
    ae: autoElement ? 1 : undefined,
    tx: [[
      [[], 0],
      [[path]],
    ]],
  };
}

function registerUnitTemplate(tag: string, meta: TemplateMeta): TemplateMeta {
  const webui = window.__webui ?? (window.__webui = {});
  const templates = webui.templates ?? (webui.templates = {});
  templates[tag] = meta;
  return meta;
}

describe('auto element fallback', () => {
  test('installAutoElementRuntime registers a CoreElement fallback for metadata roots', async () => {
    const tag = `auto-unit-${Date.now()}`;

    registerUnitTemplate(tag, textTemplate('displayValue'));
    installAutoElementRuntime();
    await new Promise<void>(resolve => queueMicrotask(resolve));

    const ctor = registry.get(tag);
    assert.ok(ctor);
    assert.deepEqual((ctor as ObservedElementConstructor).observedAttributes, ['display-value']);

    const instance = new ctor() as HTMLElement & {
      displayValue?: unknown;
      setState(state: Record<string, unknown>): void;
      attributeChangedCallback(name: string, oldValue: string | null, newValue: string | null): void;
      $emit?: (name: string, detail?: unknown) => boolean;
    };
    instance.setState({ displayValue: 'Loaded' });
    assert.equal(instance.displayValue, undefined);

    instance.attributeChangedCallback('display-value', 'Loaded', 'From attribute');
    assert.equal(instance.displayValue, undefined);
    // Auto-elements extend the static CoreElement, so interactive helpers like
    // $emit are tree-shaken away — an HTML-only fallback never needs them.
    assert.equal(typeof instance.$emit, 'undefined');
  });

  test('template state handles roots that match native HTMLElement properties', async () => {
    const tag = `auto-native-title-${Date.now()}`;

    registerUnitTemplate(tag, textTemplate('title'));
    installAutoElementRuntime();
    await new Promise<void>(resolve => queueMicrotask(resolve));

    const ctor = registry.get(tag);
    assert.ok(ctor);

    const instance = new ctor() as HTMLElement & {
      setState(state: Record<string, unknown>): void;
      attributeChangedCallback(name: string, oldValue: string | null, newValue: string | null): void;
      $resolveValue(path: string): unknown;
    };

    instance.setState({ title: 'Loaded from state' });
    assert.equal(instance.$resolveValue('title'), 'Loaded from state');

    instance.attributeChangedCallback('title', 'Loaded from state', 'Loaded from attr');
    assert.equal(instance.$resolveValue('title'), 'Loaded from attr');
  });

  test('installAutoElementRuntime does not overwrite an existing custom element', async () => {
    const tag = `existing-unit-${Date.now()}`;
    const existing = class ExistingElement extends HTMLElement {};
    customElements.define(tag, existing);

    registerUnitTemplate(tag, textTemplate('title'));
    installAutoElementRuntime();
    await new Promise<void>(resolve => queueMicrotask(resolve));
    assert.equal(customElements.get(tag), existing);
  });

  test('installAutoElementRuntime skips templates with event handlers', async () => {
    const tag = `interactive-unit-${Date.now()}`;

    registerUnitTemplate(tag, {
      h: '<button></button>',
      e: [['click', 'onClick', [], [0]]],
    });
    installAutoElementRuntime();
    await new Promise<void>(resolve => queueMicrotask(resolve));
    assert.equal(customElements.get(tag), undefined);
  });

  test('installAutoElementRuntime registers each missing template', async () => {
    const first = `auto-all-a-${Date.now()}`;
    const second = `auto-all-b-${Date.now()}`;

    registerUnitTemplate(first, textTemplate('title'));
    registerUnitTemplate(second, textTemplate('itemCount'));
    installAutoElementRuntime();
    await new Promise<void>(resolve => queueMicrotask(resolve));

    assert.ok(customElements.get(first));
    assert.ok(customElements.get(second));
  });

  test('installAutoElementRuntime only claims compiler-marked auto elements', async () => {
    const allowed = `runtime-allowed-${Date.now()}`;
    const skipped = `runtime-skipped-${Date.now()}`;
    registerUnitTemplate(allowed, textTemplate('title'));
    registerUnitTemplate(skipped, textTemplate('title', false));

    installAutoElementRuntime();
    await new Promise<void>(resolve => queueMicrotask(resolve));

    assert.ok(customElements.get(allowed));
    assert.equal(customElements.get(skipped), undefined);
  });
});
