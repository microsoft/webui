// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gzipSync } from 'node:zlib';
import { build } from 'esbuild';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', '..');
const source = (packageName: string, file: string): string =>
  JSON.stringify(resolve(root, 'packages', packageName, 'src', file));
const framework = (file: string): string => source('webui-framework', file);
const router = (file: string): string => source('webui-router', file);
const fixture = JSON.stringify(resolve(
  root, 'packages', 'webui-framework', 'tests', 'fixtures', 'basics', 'element.ts',
));

interface Bundle {
  initialBytes: number;
  initialGzip: number;
  totalBytes: number;
  totalGzip: number;
  inputs: Set<string>;
  initialInputs: Set<string>;
}

const entries = {
  buffered: `import ${fixture};`,
  styled: `import ${framework('index.ts')}; import ${fixture};`,
  templates: `export { getTemplate, registerTemplateData } from ${framework('templates-entry.ts')};`,
  progressive: `import ${framework('streaming-entry.ts')};`,
  jsonRouter: `import { WebUIRouter } from ${router('core.ts')}; new WebUIRouter().start();`,
  ndjsonRouter: `import { WebUIRouter } from ${router('core.ts')}; import { ndjson } from ${router('streaming.ts')}; new WebUIRouter({ streaming: ndjson }).start();`,
  fullRouter: `import { Router } from ${router('index.ts')}; Router.start();`,
  resources: `export { fetchComponentTemplates, readJsonPartial } from ${router('resources-entry.ts')};`,
  records: `export { NdjsonReader } from ${router('ndjson-reader.ts')};`,
  deferredJson: `export { createDeferredLoader } from ${router('deferred-entry.ts')};`,
  deferredStream: `import { createDeferredLoader } from ${router('deferred-entry.ts')}; import { deferredNdjson } from ${router('deferred-ndjson.ts')}; window.loader = createDeferredLoader({ transport: deferredNdjson });`,
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
    stdin: { contents: entries[profile], sourcefile: `${profile}.ts`, resolveDir: root, loader: 'ts' },
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
    tsconfig: resolve(root, 'packages', 'webui-framework', 'tsconfig.test.json'),
    define: { __WEBUI_DEV__: 'false' },
    logLevel: 'silent',
  });
  const outputs = result.metafile.outputs;
  const entry = Object.keys(outputs).find(path => outputs[path].entryPoint === `${profile}.ts`);
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
    inputs: new Set(), initialInputs: new Set(),
  };
  for (const [path, output] of Object.entries(outputs)) {
    const bytes = files.get(resolve(root, path));
    assert.ok(bytes);
    const gzip = gzipSync(bytes).byteLength;
    measured.totalBytes += bytes.byteLength;
    measured.totalGzip += gzip;
    if (initial.has(path)) {
      measured.initialBytes += bytes.byteLength;
      measured.initialGzip += gzip;
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

test('buffered core excludes CSS preparation and optional coordinators with a measured size reduction', async () => {
  const [core, styled] = await Promise.all([bundle('buffered'), bundle('styled')]);
  excludes(core.inputs, [
    'element/styles.ts', 'element/link-styles.ts', 'styles-entry.ts',
    'template-cache.ts', 'streaming-coordinator.ts', 'lazy-hydration-coordinator.ts',
  ]);
  assert.ok(core.initialBytes <= 66_000, `core startup: ${core.initialBytes} bytes`);
  assert.ok(core.initialGzip <= 22_000, `core startup gzip: ${core.initialGzip} bytes`);
  assert.ok(core.initialBytes < styled.initialBytes * 0.85, 'core should save at least 15% versus full style installation');
});

test('JSON-only routing excludes streaming and unselected policies from all emitted chunks', async () => {
  const [json, full] = await Promise.all([bundle('jsonRouter'), bundle('fullRouter')]);
  excludes(json.inputs, [
    'streaming.ts', 'ndjson-reader.ts', 'cache.ts', 'actions.ts', 'pending.ts',
    'preload.ts', 'prepared-preload.ts', 'route-boundary.ts', 'view-transitions.ts',
    'diagnostics.ts', 'router.ts',
  ]);
  assert.ok(json.totalBytes < full.totalBytes * 0.9, 'lean routing should exclude at least 10% of the full dependency graph');
});

test('NDJSON routing adds only the partial reader, not unrelated router policies', async () => {
  const streamed = await bundle('ndjsonRouter');
  assert.ok([...streamed.inputs].some(input => input.endsWith('/ndjson-reader.ts')));
  excludes(streamed.inputs, [
    'streaming-coordinator.ts', 'cache.ts', 'actions.ts', 'pending.ts', 'preload.ts',
    'route-boundary.ts', 'view-transitions.ts',
  ]);
});

test('resource and record consumers have no route or rendering engine dependency', async () => {
  const [resources, records, templates] = await Promise.all([
    bundle('resources'), bundle('records'), bundle('templates'),
  ]);
  for (const output of [resources, records, templates]) {
    excludes(output.inputs, [
      'router-core.ts', 'router.ts', 'chain.ts', 'route-element.ts',
      'template-element.ts', 'element.ts', 'element/styles.ts',
      'element/link-styles.ts', 'streaming-coordinator.ts',
    ]);
  }
  excludes(resources.inputs, ['streaming.ts', 'ndjson-reader.ts']);
});

test('deferred delivery stays hydration-agnostic and JSON consumers exclude NDJSON', async () => {
  const [json, streamed] = await Promise.all([bundle('deferredJson'), bundle('deferredStream')]);
  for (const output of [json, streamed]) {
    excludes(output.inputs, [
      'router-core.ts', 'router.ts', 'chain.ts', 'route-element.ts',
      'template-element.ts', 'element.ts', 'element/styles.ts',
      'streaming-coordinator.ts', 'pending.ts', 'view-transitions.ts',
    ]);
  }
  excludes(json.inputs, ['ndjson-reader.ts', 'deferred-ndjson.ts']);
  assert.ok([...streamed.inputs].some(input => input.endsWith('/ndjson-reader.ts')));
});
test('early progressive HTML remains separate from network partial decoding and authored rendering', async () => {
  const progressive = await bundle('progressive');
  excludes(progressive.inputs, ['router-core.ts', 'ndjson-reader.ts']);
  excludes(progressive.initialInputs, ['template-element.ts', 'element.ts', 'element/link-styles.ts']);
  assert.ok(progressive.initialBytes <= 32 * 1024, `early bootstrap: ${progressive.initialBytes} bytes`);
  assert.ok(progressive.initialGzip <= 11 * 1024, `early bootstrap gzip: ${progressive.initialGzip} bytes`);
});
