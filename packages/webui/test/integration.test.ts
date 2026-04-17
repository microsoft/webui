// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Integration test for the @microsoft/webui npm package.
 * Uses the built-in Node.js test runner.
 */

import { describe, test, before, after } from 'node:test';
import { strict as assert } from 'node:assert';
import { build, render, renderStream, inspect, renderComponentTemplates } from '@microsoft/webui';
import type { ComponentTemplatesResponse } from '@microsoft/webui';
import { writeFileSync, mkdtempSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

let appDir: string;

before(() => {
  appDir = mkdtempSync(join(tmpdir(), 'webui-test-'));

  writeFileSync(join(appDir, 'index.html'), `
<!DOCTYPE html>
<html>
<body>
  <h1>Hello, {{name}}!</h1>
  <for each="item in items">
    <p>{{item}}</p>
  </for>
  <if condition="show">
    <footer>Visible</footer>
  </if>
</body>
</html>
`);

  writeFileSync(join(appDir, 'my-card.html'), '<div class="card"><slot></slot></div>');
  writeFileSync(join(appDir, 'my-card.css'), '.card { border: 1px solid #ccc; }');
  writeFileSync(join(appDir, 'index2.html'), '<my-card>Hello</my-card>');
});

after(() => {
  rmSync(appDir, { recursive: true, force: true });
});

describe('build', () => {
  test('returns protocol and stats', () => {
    const result = build({ appDir });
    assert.ok(result.protocol.length > 0);
    assert.ok(result.stats.fragmentCount > 0);
    assert.ok(result.stats.durationMs >= 0);
    assert.ok(result.stats.protocolSizeBytes > 0);
    assert.ok(result.stats.componentCount >= 0);
  });

  test('emits CSS files for used components', () => {
    const result = build({ appDir, entry: 'index2.html', css: 'link' });
    assert.ok(result.stats.componentCount > 0);
    assert.equal(result.cssFiles.length, 2); // [filename, content]
    assert.equal(result.stats.cssFileCount, 1);
  });

  test('throws on missing appDir', () => {
    assert.throws(() => build({ appDir: '/nonexistent' }));
  });

  test('throws on invalid css mode', () => {
    assert.throws(() => build({ appDir, css: 'bogus' as 'link' }));
  });
});

describe('inspect', () => {
  test('returns valid JSON with fragments', () => {
    const result = build({ appDir });
    const json = inspect(result.protocol);
    const parsed = JSON.parse(json);
    assert.ok(parsed.fragments);
    assert.ok(parsed.fragments['index.html']);
  });
});

describe('render', () => {
  test('substitutes signals', () => {
    const result = build({ appDir });
    const html = render(result.protocol, { name: 'WebUI', items: ['a', 'b'], show: true });
    assert.ok(html.includes('Hello, WebUI!'));
  });

  test('expands for-loop', () => {
    const result = build({ appDir });
    const html = render(result.protocol, { name: 'X', items: ['a', 'b'], show: false });
    assert.ok(html.includes('<p>a</p>'));
    assert.ok(html.includes('<p>b</p>'));
  });

  test('includes if-true block', () => {
    const result = build({ appDir });
    const html = render(result.protocol, { name: 'X', items: [], show: true });
    assert.ok(html.includes('<footer>Visible</footer>'));
  });

  test('excludes if-false block', () => {
    const result = build({ appDir });
    const html = render(result.protocol, { name: 'X', items: [], show: false });
    assert.ok(!html.includes('<footer>'));
  });
});

describe('renderStream', () => {
  test('streams chunks via callback', () => {
    const result = build({ appDir });
    const chunks: string[] = [];
    renderStream(result.protocol, { name: 'Stream', items: ['x'], show: false }, (chunk) => {
      chunks.push(chunk);
    });
    assert.ok(chunks.length > 0);
    assert.ok(chunks.join('').includes('Hello, Stream!'));
  });
});

describe('renderComponentTemplates', () => {
  test('returns valid response shape', () => {
    const result = build({ appDir, entry: 'index2.html' });
    const json = renderComponentTemplates(result.protocol, ['my-card'], '');
    const parsed: ComponentTemplatesResponse = JSON.parse(json);
    assert.ok(Array.isArray(parsed.templates));
    assert.ok(Array.isArray(parsed.templateStyles));
    assert.equal(typeof parsed.inventory, 'string');
  });

  test('returns empty arrays for unknown component', () => {
    const result = build({ appDir });
    const json = renderComponentTemplates(result.protocol, ['nonexistent-widget'], '');
    const parsed: ComponentTemplatesResponse = JSON.parse(json);
    assert.deepEqual(parsed.templates, []);
    assert.deepEqual(parsed.templateStyles, []);
  });
});
