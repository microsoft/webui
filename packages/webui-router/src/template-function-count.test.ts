// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import './browser-shim.js';

import { strict as assert } from 'node:assert';
import { test } from 'node:test';

import { WebUIRouter } from './router.js';
import { registerTemplatesAndStyles } from './templates.js';

const TEMPLATE_FN_COUNT = Symbol.for('microsoft.webui.templateFnCount');

type TemplateFunctionRegistry = Record<string, unknown>
  & Record<symbol, number | undefined>;

interface TestRuntime {
  __webui?: {
    inventory?: string;
    templates?: Record<string, unknown>;
    templateFns?: TemplateFunctionRegistry;
  };
}

interface ScriptNode {
  nonce: string;
  textContent: string;
}

interface ScriptDocument {
  createElement(tag: string): ScriptNode;
  head: {
    appendChild(script: ScriptNode): ScriptNode;
    removeChild(script: ScriptNode): ScriptNode;
  };
}

function globals(): TestRuntime {
  return globalThis as unknown as TestRuntime;
}

function registerFunctions(templateFunctions: Record<string, string>): void {
  registerTemplatesAndStyles({ templateFunctions }, '', () => {});
}

function addConditionTemplate(name: string): unknown {
  const template = {
    h: '<!--wc:0--><!--/wc-->',
    b: [{ h: '<p>Ready</p>' }],
    c: [[[0, ['ready']], 0, [[], 0]]],
  };
  const runtime = globals().__webui;
  assert.ok(runtime);
  (runtime.templates ??= {})[name] = template;
  return template;
}

test('keeps the closure count exact across soft writes, GC, and zero deletion', async () => {
  const runtime = globals();
  const savedWebui = runtime.__webui;
  const scriptDocument = document as unknown as ScriptDocument;
  const savedCreateElement = scriptDocument.createElement;
  const savedHead = scriptDocument.head;

  scriptDocument.createElement = () => ({ nonce: '', textContent: '' });
  scriptDocument.head = {
    appendChild(script) {
      // eslint-disable-next-line no-new-func
      Function(script.textContent)();
      return script;
    },
    removeChild(script) {
      return script;
    },
  };

  try {
    runtime.__webui = {
      inventory: 'ff',
      templates: {},
      templateFns: {
        'initial-a': [() => true],
        'initial-b': [() => false],
      },
    };
    const initialA = addConditionTemplate('initial-a');
    const initialB = addConditionTemplate('initial-b');
    const { getTemplate } = await import('@microsoft/webui-framework');

    assert.strictEqual(getTemplate('initial-a'), initialA);
    let registry = runtime.__webui.templateFns;
    assert.ok(registry);
    assert.equal(registry[TEMPLATE_FN_COUNT], 1);

    registerFunctions({ 'initial-b': '[function(){return true}]' });
    registry = runtime.__webui.templateFns;
    assert.ok(registry);
    assert.equal(
      registry[TEMPLATE_FN_COUNT],
      1,
      'replacing a pending closure array must not increment the count',
    );

    registerFunctions({ 'late-route': '[function(){return true}]' });
    registry = runtime.__webui.templateFns;
    assert.ok(registry);
    assert.equal(
      registry[TEMPLATE_FN_COUNT],
      2,
      'a router write after count initialization must increment the count',
    );

    const lateRoute = addConditionTemplate('late-route');
    assert.strictEqual(getTemplate('late-route'), lateRoute);
    registry = runtime.__webui.templateFns;
    assert.ok(registry);
    assert.equal(
      registry[TEMPLATE_FN_COUNT],
      1,
      'the registry must remain while one closure array is pending',
    );
    assert.strictEqual(getTemplate('initial-b'), initialB);
    assert.equal(
      runtime.__webui.templateFns,
      undefined,
      'the registry must be deleted when the exact count reaches zero',
    );

    for (let i = 0; i < 4; i++) {
      const tag = `soft-route-${i}`;
      registerFunctions({ [tag]: '[function(){return true}]' });
      registry = runtime.__webui.templateFns;
      assert.ok(registry);
      assert.equal(registry[TEMPLATE_FN_COUNT], 1);
      const template = addConditionTemplate(tag);
      assert.strictEqual(getTemplate(tag), template);
      assert.equal(runtime.__webui.templateFns, undefined);
    }

    registerFunctions({
      'gc-a': '[function(){return true}]',
      'gc-b': '[function(){return true}]',
      'gc-c': '[function(){return true}]',
    });
    const gcA = addConditionTemplate('gc-a');
    addConditionTemplate('gc-b');
    addConditionTemplate('gc-c');
    assert.strictEqual(getTemplate('gc-a'), gcA);
    registry = runtime.__webui.templateFns;
    assert.ok(registry);
    assert.equal(registry[TEMPLATE_FN_COUNT], 2);

    const registryBeforeGc = registry;
    new WebUIRouter().gc();
    assert.strictEqual(runtime.__webui.templateFns, registryBeforeGc);
    assert.deepEqual(Object.keys(registryBeforeGc), []);
    assert.equal(
      registryBeforeGc[TEMPLATE_FN_COUNT],
      0,
      'clearing the registry must reset its non-enumerated count',
    );

    registerFunctions({ 'after-gc': '[function(){return true}]' });
    registry = runtime.__webui.templateFns;
    assert.strictEqual(registry, registryBeforeGc);
    assert.equal(registry?.[TEMPLATE_FN_COUNT], 1);
    const afterGc = addConditionTemplate('after-gc');
    assert.strictEqual(getTemplate('after-gc'), afterGc);
    assert.equal(runtime.__webui.templateFns, undefined);
  } finally {
    runtime.__webui = savedWebui;
    scriptDocument.createElement = savedCreateElement;
    scriptDocument.head = savedHead;
  }
});
