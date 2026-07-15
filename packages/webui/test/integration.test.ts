// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Integration test for the @microsoft/webui npm package.
 * Uses the built-in Node.js test runner.
 */

import { describe, test, before, after } from 'node:test';
import { strict as assert } from 'node:assert';
import {
  build,
  inspect,
  render,
  renderComponentTemplates,
  renderPartial,
  renderStream,
} from '@microsoft/webui';
import type { ComponentTemplatesResponse } from '@microsoft/webui';
import { existsSync, writeFileSync, mkdtempSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

let appDir: string;

before(() => {
  const addonName = process.platform === 'win32'
    ? 'webui_node.dll'
    : process.platform === 'darwin'
      ? 'libwebui_node.dylib'
      : 'libwebui_node.so';
  const workspaceAddon = join(process.cwd(), '..', '..', 'target', 'debug', addonName);
  if (existsSync(workspaceAddon)) {
    process.env.WEBUI_ADDON_PATH = workspaceAddon;
  }

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
  writeFileSync(join(appDir, 'app-shell.html'), '<div>{{name}}</div>');
  writeFileSync(join(appDir, 'app-shell.ts'), 'export {};');
  writeFileSync(join(appDir, 'lazy-panel.html'), '<p>{{title}}</p>');
  writeFileSync(join(appDir, 'lazy-panel.ts'), 'export {};');
  writeFileSync(join(appDir, 'index3.html'), '<app-shell></app-shell>');
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
    assert.ok(Array.isArray(result.warnings));
  });

  test('emits CSS files for used components', () => {
    const result = build({ appDir, entry: 'index2.html', css: 'link' });
    assert.ok(result.stats.componentCount > 0);
    assert.equal(result.cssFiles.length, 2); // [filename, content]
    assert.equal(result.stats.cssFileCount, 1);
  });

  test('emits static component asset files', () => {
    const result = build({
      appDir,
      entry: 'index3.html',
      plugin: 'webui',
      componentAssetRoots: ['lazy-panel'],
    });
    assert.equal(result.componentAssetFiles.length, 2); // [filename, content]
    assert.equal(result.componentAssetFiles[0], 'lazy-panel.webui.js');
    assert.match(result.componentAssetFiles[1], /webui-component-asset/);
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

  test('reuses one protocol for object and JSON string state', () => {
    const result = build({ appDir });
    const objectHtml = render(result.protocol, { name: 'Object', items: [], show: false });
    const jsonHtml = render(
      result.protocol,
      JSON.stringify({ name: 'JSON', items: [], show: false }),
    );
    assert.ok(objectHtml.includes('Hello, Object!'));
    assert.ok(jsonHtml.includes('Hello, JSON!'));
  });

  test('invalidates the prepared cache when a protocol buffer mutates', () => {
    const result = build({ appDir });
    const state = { name: 'Cache', items: [], show: true };
    const initialHtml = render(result.protocol, state);
    assert.ok(initialHtml.includes('<footer>Visible</footer>'));

    const offset = result.protocol.indexOf('Visible');
    assert.ok(offset >= 0);
    result.protocol.write('Altered', offset, 'utf8');

    const updatedHtml = render(result.protocol, state);
    assert.ok(updatedHtml.includes('<footer>Altered</footer>'));
    assert.ok(!updatedHtml.includes('<footer>Visible</footer>'));
  });

  test('does not alias an empty plugin to the omitted plugin cache entry', () => {
    const result = build({ appDir });
    const state = { name: 'Plugin', items: [], show: false };
    render(result.protocol, state);
    assert.throws(
      () => render(result.protocol, state, { plugin: '' }),
      /Unknown plugin/,
    );
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
    assert.equal(typeof parsed.templates, 'object');
    assert.ok(Array.isArray(parsed.templateStyles));
    assert.equal(typeof parsed.templateFunctions, 'object');
    assert.equal(typeof parsed.inventory, 'string');
  });

  test('returns empty template maps for unknown component', () => {
    const result = build({ appDir });
    const json = renderComponentTemplates(result.protocol, ['nonexistent-widget'], '');
    const parsed: ComponentTemplatesResponse = JSON.parse(json);
    assert.deepEqual(parsed.templates, {});
    assert.deepEqual(parsed.templateFunctions, {});
    assert.deepEqual(parsed.templateStyles, []);
  });
});

describe('renderPartial', () => {
  test('preserves full state when projection metadata is absent', () => {
    const result = build({ appDir, entry: 'index3.html', plugin: 'webui' });
    const json = renderPartial(
      result.protocol,
      '{"name":"Partial","serverOnly":"drop"}',
      'index3.html',
      '/',
      '',
    );
    const parsed = JSON.parse(json);
    assert.equal(parsed.state.name, 'Partial');
    assert.equal(parsed.state.serverOnly, 'drop');
  });
});
