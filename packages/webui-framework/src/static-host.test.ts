// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';
import type { TemplateMeta } from './template.js';

const registry = new Map<string, CustomElementConstructor>();
const windowListeners = new Map<string, Array<(event: Event) => void>>();

Object.defineProperty(globalThis, 'HTMLElement', {
  value: class HTMLElement {
    tagName = '';
    isConnected = false;
    childNodes: unknown[] = [];
    shadowRoot = null;

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

const { installTemplateElementRuntime } = await import('./static-host.js');

function registerTemplate(tag: string, meta: TemplateMeta): TemplateMeta {
  const webui = window.__webui ?? (window.__webui = {});
  const templates = webui.templates ?? (webui.templates = {});
  templates[tag] = meta;
  return meta;
}

describe('dormant template host runtime', () => {
  test('defines compiler-owned hosts without authored stubs', () => {
    const tag = `dormant-unit-${Date.now()}`;
    registerTemplate(tag, {
      h: '<p></p>',
      th: 1,
      tr: ['message'],
      ta: ['message'],
    });

    installTemplateElementRuntime();

    const ctor = registry.get(tag);
    assert.ok(ctor);
    const instance = new ctor() as HTMLElement & {
      $shouldDeferSSRHydration(): boolean;
      $shouldApplySSRBootstrapState(): boolean;
      setState(state: Record<string, unknown>): void;
    };
    assert.equal(instance.$shouldDeferSSRHydration(), true);
    assert.equal(instance.$shouldApplySSRBootstrapState(), false);
    assert.equal(typeof instance.setState, 'function');
  });

  test('defines fully static templates for client-created navigation', () => {
    const tag = `static-unit-${Date.now()}`;
    registerTemplate(tag, { h: '<p>Static</p>', th: 1 });

    installTemplateElementRuntime();

    assert.ok(registry.get(tag));
  });

  test('does not claim authored or already registered elements', () => {
    const authoredTag = `authored-unit-${Date.now()}`;
    const existingTag = `existing-unit-${Date.now()}`;
    const existing = class ExistingElement extends HTMLElement {};
    customElements.define(existingTag, existing);

    registerTemplate(authoredTag, { h: '<p></p>', tr: ['message'] });
    registerTemplate(existingTag, { h: '<p></p>', th: 1 });

    installTemplateElementRuntime();

    assert.equal(customElements.get(authoredTag), undefined);
    assert.equal(customElements.get(existingTag), existing);
  });

  test('claims templates registered after startup', () => {
    const tag = `event-unit-${Date.now()}`;
    const meta = registerTemplate(tag, { h: '<p></p>', th: 1 });

    window.dispatchEvent(new CustomEvent('webui:templates-registered', {
      detail: { templates: { [tag]: meta } },
    }));

    assert.ok(customElements.get(tag));
  });
});
