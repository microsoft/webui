// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { hydrationStart, hydrationEnd } from './lifecycle.js';
import { componentStyleRuntime } from './element/style-runtime.js';
import { catalogFor, type ComponentStyles } from './element/style-catalog.js';

test('startup installs styles before early declarations and waits for missing metadata', async t => {
  const saved = ['HTMLElement', 'window', 'document', 'customElements'].map(name =>
    [name, Object.getOwnPropertyDescriptor(globalThis, name)] as const);
  const constructors = new Map<string, CustomElementConstructor>();
  const observedBootstrap: unknown[] = [];
  const completedAt: number[] = [];
  const styles: ComponentStyles = {
    version: 1,
    strategy: 'style',
    resources: { shared: { kind: 'style', css: 'p { color: red; }' } },
    closures: { 'early-widget': ['shared'] },
  };
  let bootstrapReads = 0;
  t.mock.timers.enable({ apis: ['setTimeout'] });
  Object.defineProperties(globalThis, {
    HTMLElement: { configurable: true, value: class {} },
    window: {
      configurable: true,
      value: Object.assign(new EventTarget(), {
        __webui: {
          state: { bootstrap: 'server' },
          componentStyles: styles,
          templates: { 'early-widget': { v: 3, h: '', th: 1, tr: ['title'], ta: ['title'] } },
        },
      }),
    },
    document: {
      configurable: true,
      value: {
        readyState: 'complete',
        getElementById() { bootstrapReads++; return null; },
      },
    },
    customElements: {
      configurable: true,
      value: {
        get: (name: string) => constructors.get(name),
        define(name: string, ctor: CustomElementConstructor) {
          assert.equal(constructors.has(name), false, 'a definition is installed once');
          constructors.set(name, ctor);
          if (name === 'early-widget' || name === 'arriving-widget') {
            assert.ok(componentStyleRuntime, 'normal startup installs styles before hydration');
            hydrationStart();
            observedBootstrap.push(window.__webui?.state?.bootstrap);
            hydrationEnd();
          }
        },
      },
    },
  });
  try {
    const { WebUIElement } = await import('./index.js');
    const { startRuntime } = await import('./runtime.js');
    const { registerTemplateData } = await import('./template-registry.js');
    class EarlyWidget extends WebUIElement {}
    class ArrivingWidget extends WebUIElement {}
    class MissingWidget extends WebUIElement {}

    EarlyWidget.define('early-widget');
    ArrivingWidget.define('arriving-widget');
    MissingWidget.define('missing-widget');
    assert.equal(constructors.size, 0, 'declarations wait for explicit runtime startup');
    assert.throws(() => EarlyWidget.define('early-widget'), /already pending definition/);
    registerTemplateData({ 'arriving-widget': { v: 3, h: '', tr: ['count'], ta: ['count'] } });
    assert.equal(constructors.size, 0, 'metadata delivery must not start queued declarations');
    assert.equal(bootstrapReads, 0, 'declarations do not consume metadata before installers');

    await import('./template-cache-entry.js');
    assert.equal(componentStyleRuntime, undefined, 'pure imports do not install styles');
    assert.equal(Object.hasOwn(window, '__webuiRegisterComponentStyles'), false);
    window.addEventListener('webui:hydration-complete', () => completedAt.push(constructors.size));
    startRuntime();
    const installedStyles = componentStyleRuntime;
    const registerStyles = window.__webuiRegisterComponentStyles;
    assert.equal(typeof registerStyles, 'function');
    assert.equal(typeof window.__webuiPrepareResources, 'function');
    assert.equal(typeof window.__webuiPreloadStyles, 'function');
    await import('./styles-entry.js');
    assert.equal(componentStyleRuntime, installedStyles, 'the compatibility import shares the default installer');
    assert.equal(window.__webuiRegisterComponentStyles, registerStyles);
    assert.deepEqual(observedBootstrap, ['server', 'server'], 'queued roots retain the same initial SSR state');
    assert.deepEqual(completedAt, [2], 'startup completes only after the available declarations register');
    assert.equal(customElements.get('early-widget'), EarlyWidget);
    assert.equal(customElements.get('arriving-widget'), ArrivingWidget);
    assert.equal(customElements.get('missing-widget'), undefined);
    assert.deepEqual(Reflect.get(EarlyWidget, 'observedAttributes'), ['title']);
    assert.deepEqual(Reflect.get(ArrivingWidget, 'observedAttributes'), ['count']);

    t.mock.timers.tick(0);
    assert.equal(customElements.get('early-widget'), EarlyWidget, 'authored definitions win over static hosts');
    startRuntime();
    assert.equal(componentStyleRuntime, installedStyles, 'repeated startup shares one style runtime');
    assert.equal(window.__webuiRegisterComponentStyles, registerStyles, 'repeated startup does not allocate another bridge');
    t.mock.timers.tick(0);
    assert.equal(constructors.size, 2, 'repeated startup does not repeat definitions');

    registerTemplateData({ 'missing-widget': { v: 3, h: '', tr: ['value'], ta: ['value'] } });
    assert.equal(customElements.get('missing-widget'), MissingWidget);
    assert.deepEqual(Reflect.get(MissingWidget, 'observedAttributes'), ['value']);

    class ImmediateWidget extends WebUIElement {}
    registerTemplateData({ 'immediate-widget': { v: 3, h: '' } });
    ImmediateWidget.define('immediate-widget');
    assert.equal(customElements.get('immediate-widget'), ImmediateWidget, 'later definitions are synchronous');

    const catalog = catalogFor(document);
    assert.equal(catalog.resources.size, 1, 'SSR styles enter the shared document catalog');
    assert.ok(registerStyles);
    await registerStyles({
      ...styles, closures: { 'route-widget': ['shared'] },
    });
    const { defineComponentAsset } = await import('./component-asset-runtime.js');
    const assetStyles: ComponentStyles = {
      ...styles, closures: { 'asset-widget': ['shared'] },
    };
    await defineComponentAsset({
      type: 'webui-component-asset', version: 4, root: 'asset-widget', externalComponents: [],
      imports: [{
        templates: { 'asset-widget': { v: 3, h: '<p>Asset</p>', th: 1 } },
        componentStyles: assetStyles,
      }],
      componentStyles: assetStyles,
    }).preload();
    assert.equal(catalogFor(document), catalog, 'asset and router registration share the SSR catalog');
    assert.equal(catalog.resources.size, 1, 'shared resource IDs are not duplicated by delivery paths');
    for (const tag of ['early-widget', 'route-widget', 'asset-widget']) {
      assert.deepEqual(catalog.closures.get(tag), ['shared']);
    }
  } finally {
    for (const [name, descriptor] of saved) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor);
      else Reflect.deleteProperty(globalThis, name);
    }
  }
});
