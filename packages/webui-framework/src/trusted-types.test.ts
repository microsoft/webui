// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { configureTrustedTypes } from './trusted-types.js';
import { registerTrustedTemplateBlock } from './trusted-types-policy.js';
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
  const scripts: { textContent: Trusted; nonce?: string }[] = [];
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
      head: { appendChild(script: { textContent: Trusted; nonce?: string }) { scripts.push(script); } },
    },
  });
  return {
    parsed, scripts,
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
  assert.equal(window.__webuiTrustedTemplates, undefined);
});

test('reports blocked/duplicate browser policy creation without installing a boundary', () => {
  const f = fixture({ denied: true });
  assert.throws(() => configureTrustedTypes('app-compiled'), error =>
    error instanceof Error &&
    error.message.includes('Allow that exact name in CSP') &&
    error.cause instanceof TypeError);
  assert.equal(f.creations, 1);
  assert.equal(window.__webuiTrustedTemplates, undefined);
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
  assert.equal(Object.isFrozen(window.__webuiTrustedTemplates), true);
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

test('router compiler functions retain nonce and CSS import maps contain only serialized CSS data', () => {
  const f = fixture();
  configureTrustedTypes('app-compiled');
  const functions = { 'test-counter': '[function(v){return v("count")>0}]' };
  window.__webuiTrustedTemplates!.installTemplateFunctions(functions, 'document-nonce');
  assert.equal(f.scripts.length, 1);
  assert.equal(f.scripts[0].nonce, 'document-nonce');
  assert.equal(f.scripts[0].textContent.kind, 'script');
  assert.ok(f.scripts[0].textContent.value.includes('f["test-counter"]=[function(v)'));
  const script = document.createElement('script');
  const css = 'a::after{content:"</script><script>bad()</script>"}';
  window.__webuiTrustedTemplates!.setImportMap(script, 'component-css', css);
  const value = script.textContent as unknown as Trusted;
  assert.equal(value.kind, 'script');
  assert.deepEqual(JSON.parse(value.value), {
    imports: { 'component-css': `data:text/css,${encodeURIComponent(css)}` },
  });
});
