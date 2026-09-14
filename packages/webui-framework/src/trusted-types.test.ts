// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { configureTrustedTypes } from './trusted-types.js';
import * as publicEntry from './trusted-types.js';
import { registerTrustedTemplateBlock, setImportMapContent } from './trusted-types-policy.js';
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
  const parsed: Trusted[] = [];
  const fakeWindow = {
    trustedTypes: options.absent ? undefined : {
      createPolicy(_name: string, supplied: Rules) {
        creations++;
        if (options.denied) throw new TypeError('CSP denied name');
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
        if (tag === 'script') return { remove() {} };
        assert.equal(tag, 'template');
        return {
          content: {},
          set innerHTML(value: Trusted) {
            assert.ok(value instanceof Trusted);
            assert.equal(value.kind, 'html');
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

test('requires an explicit non-default policy name even without browser support', () => {
  fixture({ absent: true });
  for (const name of ['', 'default', '*', 'contains space', 'a;b', "'name'"]) {
    assert.throws(() => configureTrustedTypes(name), TypeError);
  }
  configureTrustedTypes('application.templates-1');
  assert.equal(window.__webuiTrustedTypesPolicyName, undefined);
});

test('reports blocked/duplicate browser policy creation without installing a boundary', () => {
  const f = fixture({ denied: true });
  assert.throws(() => configureTrustedTypes('app-compiled'), error =>
    error instanceof Error &&
    error.message.includes('Allow that exact name in CSP') &&
    error.cause instanceof TypeError);
  assert.equal(f.creations, 1);
  assert.equal(window.__webuiTrustedTypesPolicyName, undefined);
});

test('same-name configuration is idempotent; replacement and unscoped conversions fail', () => {
  const f = fixture();
  configureTrustedTypes('app-compiled');
  configureTrustedTypes('app-compiled');
  assert.equal(f.creations, 1);
  assert.throws(() => configureTrustedTypes('different'), /already configured/);
  assert.throws(() => f.rules.createHTML('<img>', {}), /Only the compiled-template runtime/);
  assert.throws(() => f.rules.createScript('alert(1)', {}), /Only the compiled-template runtime/);
  assert.equal(Object.hasOwn(f.rules, 'createScriptURL'), false);
  assert.deepEqual(Object.getOwnPropertyDescriptor(window, '__webuiTrustedTypesPolicyName'), {
    value: 'app-compiled', writable: false, enumerable: false, configurable: false,
  });
});

test('configuration exposes only inert metadata, not HTML or script policy operations', () => {
  fixture();
  const before = new Set(Reflect.ownKeys(window));
  configureTrustedTypes('app-compiled');
  const added = Reflect.ownKeys(window).filter(key => !before.has(key));
  assert.deepEqual(added, ['__webuiTrustedTypesPolicyName']);
  assert.equal(typeof window.__webuiTrustedTypesPolicyName, 'string');
  assert.equal(Object.hasOwn(window, '__webuiTrustedTemplates'), false);
  assert.deepEqual(Object.keys(publicEntry), ['configureTrustedTypes']);
});

test('an independent framework copy cannot recover policy operations through global metadata', async () => {
  const f = fixture();
  configureTrustedTypes('app-compiled');
  const duplicate: typeof import('./trusted-types-policy.js') =
    await import(new URL('./trusted-types-policy.js?independent-copy', import.meta.url).href);
  assert.throws(() => duplicate.configureTrustedTypes('app-compiled'), /Share one framework module/);
  const meta = { h: '<p>Unregistered</p>' };
  assert.throws(() => duplicate.registerTrustedTemplateBlock(meta), /Share one framework module/);
  assert.throws(() => duplicate.setTemplateContent(document.createElement('template'), meta), /Share one framework module/);
  assert.equal(f.creations, 1);
  assert.equal(f.conversions, 0);
});

test('only registered unchanged template HTML enters the sink and each template is parsed once', () => {
  const f = fixture();
  configureTrustedTypes('app-compiled');
  const meta = { h: '<p>Compiler output</p>' };
  assert.throws(() => getTemplateFragment(meta), /Unregistered or modified/);
  registerTrustedTemplateBlock(meta);
  const first = getTemplateFragment(meta);
  assert.equal(first, getTemplateFragment(meta));
  assert.equal(f.conversions, 1);
  assert.deepEqual(f.parsed.map(value => value.value), ['<p>Compiler output</p>']);

  const changed = { h: '<span>Original</span>' };
  registerTrustedTemplateBlock(changed);
  changed.h = '<img src=x onerror=alert(1)>';
  registerTrustedTemplateBlock(changed);
  assert.throws(() => getTemplateFragment(changed), /Unregistered or modified/);
  assert.equal(f.conversions, 1);
});

test('CSS import maps retain nonce and contain only serialized CSS data, not executable source', () => {
  fixture();
  configureTrustedTypes('app-compiled');
  const script = document.createElement('script');
  script.type = 'importmap';
  script.nonce = 'document-nonce';
  const css = 'a::after{content:"</script><script>bad()</script>"}';
  setImportMapContent(script, 'component-css', css, window);
  const value = script.textContent as unknown as Trusted;
  assert.equal(script.nonce, 'document-nonce');
  assert.equal(value.kind, 'script');
  assert.deepEqual(JSON.parse(value.value), {
    imports: { 'component-css': `data:text/css,${encodeURIComponent(css)}` },
  });
  const executable = document.createElement('script');
  assert.throws(() => setImportMapContent(executable, 'component-css', css, window), /type="importmap"/);
  assert.equal(executable.textContent, undefined);
});

test('CSS import maps retain the string path on browsers without Trusted Types', () => {
  fixture({ absent: true });
  configureTrustedTypes('app-compiled');
  const script = document.createElement('script');
  script.type = 'importmap';
  setImportMapContent(script, 'component-css', 'p{color:red}', window);
  assert.equal(script.textContent, '{"imports":{"component-css":"data:text/css,p%7Bcolor%3Ared%7D"}}');
});
