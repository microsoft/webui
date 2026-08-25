// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';
import type { TemplateMeta } from './template.js';

const registry = new Map<string, CustomElementConstructor>();
const windowListeners = new Map<string, Array<(event: Event) => void>>();
const headChildren: FakeStyleElement[] = [];

class FakeStyleElement {
  localName: string;
  nonce = '';
  textContent = '';
  private readonly attributes = new Map<string, string>();

  constructor(localName: string) {
    this.localName = localName;
  }

  getAttribute(name: string): string | null {
    return this.attributes.get(name) ?? null;
  }

  setAttribute(name: string, value: string): void {
    this.attributes.set(name, value);
  }
}

const fakeHead = {
  get childElementCount(): number {
    return headChildren.length;
  },
  get children(): FakeStyleElement[] {
    return headChildren;
  },
  insertBefore(
    child: FakeStyleElement,
    before: FakeStyleElement | null,
  ): FakeStyleElement {
    const index = before ? headChildren.indexOf(before) : -1;
    if (index < 0) headChildren.push(child);
    else headChildren.splice(index, 0, child);
    return child;
  },
  querySelectorAll(): FakeStyleElement[] {
    return headChildren.filter(
      (child) => child.getAttribute('data-webui-resource') !== null,
    );
  },
};

Object.defineProperty(globalThis, 'HTMLElement', {
  value: class HTMLElement {
    tagName = '';
    localName = '';
    isConnected = false;
    childNodes: unknown[] = [];
    shadowRoot = null;
    parentElement = null;
    ownerDocument = document;

    hasAttribute(_name: string): boolean {
      return false;
    }

    getRootNode(): Document {
      return this.ownerDocument;
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
    nodeType: 9,
    readyState: 'complete',
    head: fakeHead,
    createElement(name: string) {
      return new FakeStyleElement(name);
    },
    getElementById() {
      return null;
    },
    querySelector() {
      return null;
    },
    querySelectorAll() {
      return fakeHead.querySelectorAll();
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
const { deferTemplateDefinition } = await import('./template.js');
const { TemplateElement } = await import('./template-element.js');
const {
  ACTIVATION_STATIC_HOST_OPT_OUT,
  STREAMING_BOUNDARY_ACTIVATE,
} = await import('./streaming-mode.js');
const { registerComponentStyles } = await import('./element/styles.js');

interface ComplexPropertyWriter {
  $writeComplexProperty(
    element: Element,
    name: string,
    value: unknown,
    replayAfterHydration: boolean,
  ): void;
}

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
      $shouldActivateOnBoundaryCommit(): boolean;
      setState(state: Record<string, unknown>): void;
    };
    assert.equal(instance.$shouldDeferSSRHydration(), true);
    assert.equal(instance.$shouldApplySSRBootstrapState(), false);
    assert.equal(
      instance.$shouldActivateOnBoundaryCommit(),
      false,
      'a compiler-owned host must stay dormant on a streaming boundary commit; only a client state write wakes it',
    );
    assert.equal(typeof instance.setState, 'function');
  });

  test('defines fully static templates for client-created navigation', () => {
    const tag = `static-unit-${Date.now()}`;
    registerTemplate(tag, { h: '<p>Static</p>', th: 1 });

    installTemplateElementRuntime();

    const ctor = registry.get(tag);
    assert.ok(ctor);
    assert.equal(ctor.prototype instanceof TemplateElement, true);
  });

  test('defines empty compiler-owned hosts without TemplateElement state', () => {
    const tag = `empty-static-unit-${Date.now()}`;
    registerTemplate(tag, { h: '', th: 1 });

    installTemplateElementRuntime();

    const ctor = registry.get(tag);
    assert.ok(ctor);
    assert.equal(ctor.prototype instanceof TemplateElement, false);
    const instance = new ctor() as HTMLElement & {
      [STREAMING_BOUNDARY_ACTIVATE](): number;
      setState?: unknown;
    };
    assert.deepEqual(
      Object.keys(instance),
      Object.keys(new HTMLElement()),
      'the minimal class adds no per-instance fields',
    );
    assert.equal(instance.setState, undefined);
    assert.equal(
      instance[STREAMING_BOUNDARY_ACTIVATE](),
      ACTIVATION_STATIC_HOST_OPT_OUT,
    );
  });

  test('preserves parent properties queued before an empty host upgrades', () => {
    const tag = `empty-pending-property-${Date.now()}`;
    registerTemplate(tag, { h: '', th: 1 });
    const parent = new TemplateElement();
    const child = new HTMLElement() as HTMLElement & {
      later?: { answer: number };
      payload?: { answer: number };
    };
    (child as unknown as { localName: string }).localName = tag;
    const ownKeysBefore = Object.keys(child);
    const payload = { answer: 42 };

    (parent as unknown as ComplexPropertyWriter).$writeComplexProperty(
      child,
      'payload',
      payload,
      false,
    );
    assert.equal(Object.hasOwn(child, 'payload'), false);

    installTemplateElementRuntime();
    const ctor = registry.get(tag);
    assert.ok(ctor);
    Object.setPrototypeOf(child, ctor.prototype);
    (child as HTMLElement & { connectedCallback(): void }).connectedCallback();

    assert.equal(child.payload, payload);
    assert.deepEqual(
      Object.keys(child).filter((key) => !ownKeysBefore.includes(key)),
      ['payload'],
      'only the queued key becomes instance state',
    );

    (parent as unknown as ComplexPropertyWriter).$writeComplexProperty(
      child,
      'later',
      { answer: 43 },
      false,
    );
    assert.deepEqual(child.later, { answer: 43 });
  });

  test('installs styles for connected and streamed empty hosts', () => {
    const connectedTag = `empty-style-connected-${Date.now()}`;
    const streamedTag = `empty-style-streamed-${Date.now()}`;
    const connectedResource = `${connectedTag}-css`;
    const streamedResource = `${streamedTag}-css`;
    registerComponentStyles({
      version: 1,
      strategy: 'style',
      resources: {
        [connectedResource]: { kind: 'style', css: ':host{display:block}' },
        [streamedResource]: { kind: 'style', css: ':host{display:none}' },
      },
      closures: {
        [connectedTag]: [connectedResource],
        [streamedTag]: [streamedResource],
      },
    });
    registerTemplate(connectedTag, { h: '', th: 1 });
    registerTemplate(streamedTag, { h: '', th: 1 });

    installTemplateElementRuntime();

    const connectedCtor = registry.get(connectedTag);
    const streamedCtor = registry.get(streamedTag);
    assert.ok(connectedCtor);
    assert.ok(streamedCtor);
    const connected = new connectedCtor() as HTMLElement & {
      connectedCallback(): void;
    };
    const streamed = new streamedCtor() as HTMLElement & {
      [STREAMING_BOUNDARY_ACTIVATE](): number;
    };
    connected.connectedCallback();
    assert.equal(
      streamed[STREAMING_BOUNDARY_ACTIVATE](),
      ACTIVATION_STATIC_HOST_OPT_OUT,
    );

    const installed = headChildren.map(
      (child) => child.getAttribute('data-webui-resource'),
    );
    assert.ok(installed.includes(connectedResource));
    assert.ok(installed.includes(streamedResource));
  });

  test('keeps behavior-bearing empty templates on TemplateElement', () => {
    const cases: Array<[string, Partial<TemplateMeta>]> = [
      ['text', { tx: [[[0, 0], [['message']]]] }],
      ['attribute', { a: [['title', 1, 'title']] }],
      ['conditional', { c: [[[() => true, []], 0, [0, 0]]] }],
      ['repeat', { r: [['items', 'item', 0, [0, 0]]] }],
      ['element event', { eg: [['click', [['handle', [], 0]]]] }],
      ['block', { b: [{ h: '' }] }],
      ['root event', { re: [['click', 'handle', []]] }],
      ['state root', { tr: ['message'] }],
      ['shadow root', { sd: 1 }],
    ];

    for (let i = 0; i < cases.length; i++) {
      const [name, behavior] = cases[i];
      const tag = `empty-${name.replace(' ', '-')}-${Date.now()}-${i}`;
      registerTemplate(tag, { h: '', th: 1, ...behavior });

      installTemplateElementRuntime();

      const ctor = registry.get(tag);
      assert.ok(ctor);
      assert.equal(
        ctor.prototype instanceof TemplateElement,
        true,
        `${name} metadata requires TemplateElement`,
      );
    }
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

  test('router-style registration defines a pending authored class before a static host can claim it', () => {
    const tag = `router-authored-unit-${Date.now()}`;
    const authored = class AuthoredRouteElement extends HTMLElement {};
    const meta = registerTemplate(tag, { h: '<p></p>', th: 1 });
    deferTemplateDefinition(tag, authored, () => customElements.define(tag, authored));

    window.dispatchEvent(new CustomEvent('webui:templates-registered', {
      detail: { templates: { [tag]: meta } },
    }));

    assert.equal(customElements.get(tag), authored);
  });

  test('does not claim tags reserved for authored lazy loaders', () => {
    const tag = `loader-unit-${Date.now()}`;
    const meta = registerTemplate(tag, { h: '<p></p>', th: 1 });
    window.__webui!.templateHostExclusions = new Set([tag]);

    window.dispatchEvent(new CustomEvent('webui:templates-registered', {
      detail: { templates: { [tag]: meta } },
    }));

    assert.equal(customElements.get(tag), undefined);
    const authored = class AuthoredLazyElement extends HTMLElement {};
    customElements.define(tag, authored);
    assert.equal(customElements.get(tag), authored);
  });
});
