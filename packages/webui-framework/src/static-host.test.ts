// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';
import type { TemplateMeta } from './template.js';

const registry = new Map<string, CustomElementConstructor>();
const windowListeners = new Map<string, Array<(event: Event) => void>>();

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

Object.defineProperty(globalThis, 'CustomEvent', {
  value: class CustomEvent<T = unknown> extends Event {
    detail: T;

    constructor(type: string, init?: CustomEventInit<T>) {
      super(type);
      this.detail = init?.detail as T;
    }
  },
  configurable: true,
});

Object.defineProperty(globalThis, 'window', {
  value: {
    __webui: { templates: {} },
    addEventListener(type: string, listener: (event: Event) => void) {
      const listeners = windowListeners.get(type);
      if (listeners) {
        listeners.push(listener);
      } else {
        windowListeners.set(type, [listener]);
      }
    },
    dispatchEvent(event: Event): boolean {
      const listeners = windowListeners.get(event.type);
      if (!listeners) return true;
      for (let i = 0; i < listeners.length; i++) listeners[i](event);
      return true;
    },
  },
  configurable: true,
});

const {
  installTemplateElementRuntime,
} = await import('./static-host.js');

type ObservedElementConstructor = CustomElementConstructor & {
  readonly observedAttributes: readonly string[];
};

function textTemplate(path: string, staticHost = true): TemplateMeta {
  const attr = path.replace(/[A-Z]/g, value => `-${value.toLowerCase()}`);
  return {
    h: '<p></p>',
    th: staticHost ? 1 : undefined,
    tr: [path],
    ta: [attr],
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

describe('static template host runtime', () => {
  test('installTemplateElementRuntime registers a TemplateElement fallback for metadata roots', async () => {
    const tag = `auto-unit-${Date.now()}`;

    registerUnitTemplate(tag, textTemplate('displayValue'));
    installTemplateElementRuntime();
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
    // Static hosts extend TemplateElement, so interactive helpers like $emit are
    // tree-shaken away - an HTML-only fallback never needs them.
    assert.equal(typeof instance.$emit, 'undefined');
  });

  test('template state handles roots that match native HTMLElement properties', async () => {
    const tag = `auto-native-title-${Date.now()}`;

    registerUnitTemplate(tag, textTemplate('title'));
    installTemplateElementRuntime();
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

  test('installTemplateElementRuntime does not overwrite an existing custom element', async () => {
    const tag = `existing-unit-${Date.now()}`;
    const existing = class ExistingElement extends HTMLElement {};
    customElements.define(tag, existing);

    registerUnitTemplate(tag, textTemplate('title'));
    installTemplateElementRuntime();
    await new Promise<void>(resolve => queueMicrotask(resolve));
    assert.equal(customElements.get(tag), existing);
  });

  test('installTemplateElementRuntime only claims compiler-owned static hosts', async () => {
    const tag = `interactive-unit-${Date.now()}`;

    registerUnitTemplate(tag, {
      h: '<button></button>',
      eg: [['click', [['onClick', [], [0]]]]],
    });
    installTemplateElementRuntime();
    await new Promise<void>(resolve => queueMicrotask(resolve));
    assert.equal(customElements.get(tag), undefined);
  });

  test('installTemplateElementRuntime registers each missing template', async () => {
    const first = `auto-all-a-${Date.now()}`;
    const second = `auto-all-b-${Date.now()}`;

    registerUnitTemplate(first, textTemplate('title'));
    registerUnitTemplate(second, textTemplate('itemCount'));
    installTemplateElementRuntime();
    await new Promise<void>(resolve => queueMicrotask(resolve));

    assert.ok(customElements.get(first));
    assert.ok(customElements.get(second));
  });

  test('installTemplateElementRuntime only claims compiler-marked static hosts', async () => {
    const allowed = `runtime-allowed-${Date.now()}`;
    const skipped = `runtime-skipped-${Date.now()}`;
    registerUnitTemplate(allowed, textTemplate('title'));
    registerUnitTemplate(skipped, textTemplate('title', false));

    installTemplateElementRuntime();
    await new Promise<void>(resolve => queueMicrotask(resolve));

    assert.ok(customElements.get(allowed));
    assert.equal(customElements.get(skipped), undefined);
  });

  test('installTemplateElementRuntime skips fully static scriptless templates', async () => {
    const tag = `static-only-${Date.now()}`;
    registerUnitTemplate(tag, {
      h: '<p>Static content</p>',
    });

    installTemplateElementRuntime();
    await new Promise<void>(resolve => queueMicrotask(resolve));

    assert.equal(customElements.get(tag), undefined);
  });

  test('template registration event claims compiler-owned static hosts', async () => {
    const tag = `event-owned-${Date.now()}`;
    const meta = registerUnitTemplate(tag, textTemplate('message'));

    installTemplateElementRuntime();
    window.dispatchEvent(new CustomEvent('webui:templates-registered', {
      detail: {
        templates: { [tag]: meta },
      },
    }));
    await new Promise<void>(resolve => queueMicrotask(resolve));

    assert.ok(customElements.get(tag));
  });
});
