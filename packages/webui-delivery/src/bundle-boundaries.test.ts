// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { mkdirSync, writeFileSync } from 'node:fs';
import { brotliCompressSync, gzipSync } from 'node:zlib';
import { build } from 'esbuild';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', '..');
const consumerRoot = process.env.WEBUI_PACKAGE_TEST_ROOT
  ? resolve(process.env.WEBUI_PACKAGE_TEST_ROOT)
  : resolve(root, 'packages', 'webui-delivery');
const component = `import { WebUIElement } from '@microsoft/webui-framework'; window.Widget = class extends WebUIElement {};`;
const renderer = `import { startRuntime } from '@microsoft/webui-framework/runtime.js'; ${component} startRuntime();`;

interface Bundle {
  initialBytes: number;
  initialGzip: number;
  totalBytes: number;
  totalGzip: number;
  initialBrotli: number;
  totalBrotli: number;
  initialRequests: number;
  chunks: number;
  inputs: Set<string>;
  initialInputs: Set<string>;
  initialText: string;
}

const entries = {
  buffered: renderer,
  styled: `import '@microsoft/webui-framework/styles.js'; ${renderer}`,
  templates: `export { getTemplate, registerTemplateData } from '@microsoft/webui-framework/templates.js';`,
  progressive: `import '@microsoft/webui-framework/streaming.js';`,
  jsonRouter: `import { WebUIRouter } from '@microsoft/webui-delivery/router.js'; new WebUIRouter().start();`,
  ndjsonRouter: `import { WebUIRouter } from '@microsoft/webui-delivery/router.js'; import { ndjson } from '@microsoft/webui-delivery/resources.js'; new WebUIRouter().start({ transport: ndjson() });`,
  fullRouter: `import { WebUIRouter, navigationCache, routeActions, routePreload, routeBoundaries, viewTransitions, routeDiagnostics } from '@microsoft/webui-delivery/router.js'; import { ndjson } from '@microsoft/webui-delivery/resources.js'; new WebUIRouter().start({ transport: ndjson(), cache: navigationCache(), actions: routeActions(), preload: routePreload(), boundaries: routeBoundaries, transitions: viewTransitions, diagnostics: routeDiagnostics });`,
  resources: `export { fetchComponentTemplates, readJsonPartial } from '@microsoft/webui-delivery/resources.js';`,
  records: `export { ndjson } from '@microsoft/webui-delivery/resources.js';`,
  lazyRegions: `import { defineLazyScripts } from '@microsoft/webui-framework/lazy.js'; defineLazyScripts({}); ${renderer}`,
  combined: `import { WebUIRouter } from '@microsoft/webui-delivery/router.js'; import '@microsoft/webui-framework/lazy.js'; ${renderer} new WebUIRouter().start();`,
  queryHelper: `export { parseQuery } from '@microsoft/webui-delivery/router.js';`,
  decoratorOnly: `export { observable } from '@microsoft/webui-framework';`,
};
type Profile = keyof typeof entries;
const bundles = new Map<Profile, Promise<Bundle>>();

function bundle(profile: Profile): Promise<Bundle> {
  let pending = bundles.get(profile);
  if (!pending) {
    pending = buildProfile(profile);
    bundles.set(profile, pending);
  }
  return pending;
}

async function buildProfile(profile: Profile): Promise<Bundle> {
  const result = await build({
    absWorkingDir: root,
    stdin: {
      contents: entries[profile],
      sourcefile: `${profile}.ts`,
      resolveDir: consumerRoot,
      loader: 'ts',
    },
    outdir: resolve(root, 'target', 'runtime-profile-tests', profile),
    bundle: true,
    splitting: true,
    format: 'esm',
    minify: true,
    treeShaking: true,
    write: false,
    metafile: true,
    platform: 'browser',
    target: 'es2022',
    supported: { 'import-attributes': true },
    tsconfig: resolve(root, 'tsconfig.json'),
    define: { __WEBUI_DEV__: 'false' },
    logLevel: 'silent',
  });
  const outputs = result.metafile.outputs;
  const entry = Object.keys(outputs).find(path =>
    outputs[path].entryPoint?.replaceAll('\\', '/').endsWith(`/${profile}.ts`));
  assert.ok(entry, 'the application entry must be present');
  const initial = new Set<string>();
  const pending = [entry];
  while (pending.length > 0) {
    const path = pending.pop()!;
    if (initial.has(path)) continue;
    initial.add(path);
    for (const edge of outputs[path].imports) {
      if (!edge.external && edge.kind !== 'dynamic-import') pending.push(edge.path);
    }
  }
  const files = new Map(result.outputFiles.map(file => [resolve(file.path), file.contents]));
  const measured: Bundle = {
    initialBytes: 0, initialGzip: 0, totalBytes: 0, totalGzip: 0,
    initialBrotli: 0, totalBrotli: 0, initialRequests: initial.size, chunks: Object.keys(outputs).length,
    inputs: new Set(), initialInputs: new Set(), initialText: '',
  };
  for (const [path, output] of Object.entries(outputs)) {
    const bytes = files.get(resolve(root, path));
    assert.ok(bytes);
    const gzip = gzipSync(bytes).byteLength;
    const brotli = brotliCompressSync(bytes).byteLength;
    measured.totalBytes += bytes.byteLength;
    measured.totalGzip += gzip;
    measured.totalBrotli += brotli;
    if (initial.has(path)) {
      measured.initialBytes += bytes.byteLength;
      measured.initialGzip += gzip;
      measured.initialBrotli += brotli;
      measured.initialText += new TextDecoder().decode(bytes);
    }
    for (const [input, contribution] of Object.entries(output.inputs)) {
      if (contribution.bytesInOutput === 0) continue;
      const normalized = input.replaceAll('\\', '/');
      measured.inputs.add(normalized);
      if (initial.has(path)) measured.initialInputs.add(normalized);
    }
  }
  return measured;
}

function excludes(inputs: ReadonlySet<string>, files: readonly string[]): void {
  for (const file of files) {
    assert.equal(
      [...inputs].some(input => input.endsWith(file)),
      false,
      `${file} must contribute no bytes to this composition`,
    );
  }
}

test('public helper imports do not retain navigation or rendering implementations', async () => {
  const [query, decorator] = await Promise.all([bundle('queryHelper'), bundle('decoratorOnly')]);
  excludes(query.inputs, [
    'router-core.js', 'streaming.js', 'ndjson-reader.js',
    'cache.js', 'actions.js', 'pending.js', 'preload.js', 'view-transitions.js',
  ]);
  excludes(decorator.inputs, [
    'template-element.js', 'element.js', 'static-host.js',
    'element/styles.js', 'element/link-styles.js', 'styles-entry.js',
  ]);
});

test('buffered core excludes CSS preparation and optional coordinators with a measured size reduction', async () => {
  const [core, styled] = await Promise.all([bundle('buffered'), bundle('styled')]);
  excludes(core.inputs, [
    'element/styles.js', 'element/link-styles.js', 'styles-entry.js', 'asset-base.js',
    'template-cache.js', 'streaming-coordinator.js', 'lazy-hydration-coordinator.js',
  ]);
  assert.ok(core.initialBytes <= 66_000, `core startup: ${core.initialBytes} bytes`);
  assert.ok(core.initialGzip <= 22_000, `core startup gzip: ${core.initialGzip} bytes`);
  assert.ok(core.initialBytes < styled.initialBytes * 0.85, 'core should save at least 15% versus full style installation');
  // The CSS module loader imports a runtime specifier; bundling must leave it native.
  assert.match(styled.initialText, /import\([\w$]+,\{with:\{type:"css"\}\}\)/);
});

test('JSON-only routing excludes streaming and unselected policies from all emitted chunks', async () => {
  const [json, full] = await Promise.all([bundle('jsonRouter'), bundle('fullRouter')]);
  excludes(json.inputs, [
    'ndjson.js', 'ndjson-reader.js', 'cache.js', 'actions.js', 'pending.js',
    'preload.js', 'prepared-preload.js', 'route-boundary.js', 'view-transitions.js',
    'diagnostics.js',
  ]);
  assert.ok(json.totalBytes < full.totalBytes * 0.9, 'lean routing should exclude at least 10% of the full dependency graph');
});

test('NDJSON routing adds only the partial reader, not unrelated router policies', async () => {
  const streamed = await bundle('ndjsonRouter');
  assert.ok([...streamed.inputs].some(input => input.endsWith('/ndjson-reader.js')));
  excludes(streamed.inputs, [
    'streaming-coordinator.js', 'cache.js', 'actions.js', 'pending.js', 'preload.js',
    'route-boundary.js', 'view-transitions.js',
  ]);
});

test('resource and record consumers have no route or rendering engine dependency', async () => {
  const [resources, records, templates] = await Promise.all([
    bundle('resources'), bundle('records'), bundle('templates'),
  ]);
  for (const output of [resources, records, templates]) {
    excludes(output.inputs, [
      'router-core.js', 'chain.js', 'route-element.js',
      'template-element.js', 'element.js', 'element/styles.js',
      'element/link-styles.js', 'streaming-coordinator.js',
    ]);
  }
  excludes(resources.inputs, ['streaming.js', 'ndjson-reader.js']);
});

test('lazy regions add one small framework runtime and no delivery code', async () => {
  const [core, lazy] = await Promise.all([bundle('buffered'), bundle('lazyRegions')]);
  assert.equal(
    [...lazy.inputs].some(input => /\/webui-delivery\/(dist|src)\//.test(input)),
    false,
    'lazy regions must not depend on routing delivery',
  );
  assert.ok([...lazy.initialInputs].some(input => input.endsWith('/lazy-entry.js')));
  excludes(core.inputs, ['lazy-entry.js', 'lazy-runtime.js', 'lazy-scripts.js', 'asset-base.js', 'speculation.js']);
  assert.equal(lazy.initialText.includes('defined at startup'), false, 'the startup guard is development-only');
  assert.equal(lazy.initialText.includes('no script defines'), false, 'the missing-script warning is development-only');
  // The rendering core holds no lazy logic, so this is the whole cost of the
  // feature, including script loading through `defineLazyScripts()`.
  const added = lazy.initialBrotli - core.initialBrotli;
  assert.ok(added <= 2_600, `lazy runtime adds ${added} brotli bytes`);
});
test('early progressive HTML remains separate from network partial decoding and authored rendering', async () => {
  const progressive = await bundle('progressive');
  excludes(progressive.inputs, ['router-core.js', 'ndjson-reader.js']);
  excludes(progressive.initialInputs, ['template-element.js', 'element.js', 'element/link-styles.js']);
  assert.ok(progressive.initialBytes <= 32 * 1024, `early bootstrap: ${progressive.initialBytes} bytes`);
  assert.ok(progressive.initialGzip <= 11 * 1024, `early bootstrap gzip: ${progressive.initialGzip} bytes`);
});

test('combined delivery reuses shared code and records public bundle measurements', async () => {
  const names = Object.keys(entries) as Profile[];
  const profiles = await Promise.all(names.map(bundle));
  const combined = await bundle('combined');
  for (const file of ['/router-core.js', '/lazy-entry.js', '/template-element.js']) {
    assert.ok([...combined.inputs].some(input => input.endsWith(file)), `${file} is bundled once`);
  }
  const output = resolve(root, 'target', process.env.WEBUI_PACKAGE_TEST_ROOT
    ? 'delivery-packed-public.json' : 'delivery-public-after.json');
  mkdirSync(dirname(output), { recursive: true });
  writeFileSync(output, JSON.stringify(Object.fromEntries(names.map((name, index) => {
    const { inputs, initialInputs, initialText: _text, ...sizes } = profiles[index];
    return [name, { ...sizes, inputs: [...inputs], initialInputs: [...initialInputs] }];
  })), null, 2));
});
