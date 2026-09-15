// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { registerTrustedTemplateBlock, setImportMapContent, setTemplateContent } from './trusted-types-policy.js';
import { getTemplateFragment } from './template-content.js';

interface Rules {
  createHTML(input: string, capability: object): string;
  createScript(input: string, capability: object): string;
}

class Trusted {
  constructor(readonly value: string, readonly kind: 'html' | 'script') {}
  toString(): string { return this.value; }
}

function fixture(options: { absent?: boolean; denied?: boolean } = {}) {
  let rules: Rules | undefined;
  let creations = 0;
  let conversions = 0;
  const parsed: (Trusted | string)[] = [];
  const names = new Set<string>();
  const fakeWindow = {
    trustedTypes: options.absent ? undefined : {
      createPolicy(name: string, supplied: Rules) {
        creations++;
        assert.equal(name, 'webui');
        if (options.denied || names.has(name)) throw new TypeError('CSP denied name');
        names.add(name);
        rules = supplied;
        return {
          createHTML(input: string, capability: object) {
            conversions++;
            return new Trusted(supplied.createHTML(input, capability), 'html');
          },
          createScript(input: string, capability: object) {
            return new Trusted(supplied.createScript(input, capability), 'script');
          },
        };
      },
    },
  };
  Object.defineProperty(globalThis, 'window', { value: fakeWindow, configurable: true });
  Object.defineProperty(globalThis, 'document', {
    configurable: true,
    value: {
      createElement(tag: string) {
        if (tag === 'script') return { textContent: '', remove() {} };
        assert.equal(tag, 'template');
        return {
          content: {},
          set innerHTML(value: Trusted | string) {
            if (options.absent) assert.equal(typeof value, 'string');
            else {
              assert.ok(value instanceof Trusted);
              assert.equal(value.kind, 'html');
            }
            parsed.push(value);
          },
        };
      },
    },
  });
  return {
    parsed,
    get rules() { return rules!; },
    get creations() { return creations; },
    get conversions() { return conversions; },
  };
}

test('registration is lazy and policy denial propagates without a string fallback', () => {
  const f = fixture({ denied: true });
  const meta = { h: '<p>Compiler output</p>' };
  registerTrustedTemplateBlock(meta);
  assert.equal(f.creations, 0);
  assert.throws(() => getTemplateFragment(meta), error =>
    error instanceof Error &&
    error.message.includes('Allow "webui" in CSP') &&
    error.cause instanceof TypeError);
  assert.equal(f.creations, 1);
  assert.equal(f.conversions, 0);
  assert.deepEqual(f.parsed, []);
});

test('the fixed-name policy is created automatically once and rejects unscoped conversions', () => {
  const f = fixture();
  assert.equal(f.creations, 0);
  for (const meta of [{ h: '<p>First</p>' }, { h: '<p>Second</p>' }]) {
    registerTrustedTemplateBlock(meta);
    getTemplateFragment(meta);
  }
  assert.equal(f.creations, 1);
  assert.equal(f.conversions, 2);
  assert.throws(() => f.rules.createHTML('<img>', {}), /Only the compiled-template runtime/);
  assert.throws(() => f.rules.createScript('alert(1)', {}), /Only the compiled-template runtime/);
  assert.equal(Object.hasOwn(f.rules, 'createScriptURL'), false);
});

test('automatic policy creation exposes no global state or trust operations', () => {
  fixture();
  const before = Reflect.ownKeys(window);
  const meta = { h: '<p>Compiler output</p>' };
  registerTrustedTemplateBlock(meta);
  getTemplateFragment(meta);
  assert.deepEqual(Reflect.ownKeys(window), before);
});

test('an independent framework copy cannot recover an existing policy or bypass duplicate-name denial', async () => {
  const f = fixture();
  const first = { h: '<p>First</p>' };
  registerTrustedTemplateBlock(first);
  getTemplateFragment(first);
  const duplicate: typeof import('./trusted-types-policy.js') =
    await import(new URL('./trusted-types-policy.js?independent-copy', import.meta.url).href);
  const meta = { h: '<p>Second</p>' };
  duplicate.registerTrustedTemplateBlock(meta);
  assert.throws(() => duplicate.setTemplateContent(document.createElement('template'), meta), /share one framework module/);
  assert.equal(f.creations, 2);
  assert.equal(f.conversions, 1);
});

test('only registered unchanged template HTML enters the sink and each template is parsed once', () => {
  const f = fixture();
  const meta = { h: '<p>Compiler output</p>' };
  assert.throws(() => getTemplateFragment(meta), /Unregistered or modified/);
  registerTrustedTemplateBlock(meta);
  const first = getTemplateFragment(meta);
  assert.equal(first, getTemplateFragment(meta));
  assert.equal(f.conversions, 1);
  assert.deepEqual(f.parsed.map(String), ['<p>Compiler output</p>']);

  const changed = { h: '<span>Original</span>' };
  registerTrustedTemplateBlock(changed);
  changed.h = '<img src=x onerror=alert(1)>';
  registerTrustedTemplateBlock(changed);
  assert.throws(() => getTemplateFragment(changed), /Unregistered or modified/);
  assert.equal(f.conversions, 1);
});

test('registered empty templates remain valid compiler output', () => {
  const f = fixture();
  const meta = { h: '' };
  registerTrustedTemplateBlock(meta);
  assert.equal(getTemplateFragment(meta), getTemplateFragment(meta));
  assert.deepEqual(f.parsed.map(String), ['']);
  assert.equal(f.conversions, 1);
});

test('each document receives its own private policy', () => {
  const first = fixture();
  const meta = { h: '<p>Compiler output</p>' };
  registerTrustedTemplateBlock(meta);
  setTemplateContent(document.createElement('template'), meta);
  const second = fixture();
  registerTrustedTemplateBlock(meta);
  setTemplateContent(document.createElement('template'), meta);
  assert.equal(first.creations, 1);
  assert.equal(second.creations, 1);
  assert.equal(first.conversions, 1);
  assert.equal(second.conversions, 1);
});

test('CSS import maps retain nonce and contain only serialized CSS data, not executable source', () => {
  const f = fixture();
  const script = document.createElement('script');
  script.type = 'importmap';
  script.nonce = 'document-nonce';
  const css = 'a::after{content:"</script><script>bad()</script>"}';
  setImportMapContent(script, 'component-css', css, window);
  assert.equal(f.creations, 1);
  const value = script.textContent as unknown as Trusted;
  assert.equal(script.nonce, 'document-nonce');
  assert.equal(value.kind, 'script');
  assert.deepEqual(JSON.parse(value.value), {
    imports: { 'component-css': `data:text/css,${encodeURIComponent(css)}` },
  });
  const executable = document.createElement('script');
  assert.throws(() => setImportMapContent(executable, 'component-css', css, window), /type="importmap"/);
  assert.equal(executable.textContent, '');
});

test('template HTML and CSS import maps retain the string path without browser support', () => {
  const f = fixture({ absent: true });
  const meta = { h: '<p>Compiler output</p>' };
  registerTrustedTemplateBlock(meta);
  getTemplateFragment(meta);
  assert.deepEqual(f.parsed, [meta.h]);
  const script = document.createElement('script');
  script.type = 'importmap';
  setImportMapContent(script, 'component-css', 'p{color:red}', window);
  assert.equal(script.textContent, '{"imports":{"component-css":"data:text/css,p%7Bcolor%3Ared%7D"}}');
  assert.equal(f.creations, 0);
});
