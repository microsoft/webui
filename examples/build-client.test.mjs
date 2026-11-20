// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import { mkdtemp, mkdir, writeFile, readFile, realpath, rm } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { test } from 'node:test';
import { setTimeout as delay } from 'node:timers/promises';
import { runInNewContext } from 'node:vm';
import { runWebUIBuild, runWebUIClientBuild } from './build-client.mjs';
import { minifyClient } from './minify-client.mjs';
import { parallelImportPlans } from './integration/lazy-ui-bench/ntp/build.mjs';

const packageRoot = fileURLToPath(new URL('../packages/webui/', import.meta.url));
const workspaceRoot = fileURLToPath(new URL('../', import.meta.url));
const require = createRequire(new URL('../packages/webui-test-support/package.json', import.meta.url));
const hashContent = contents => `sha256:${createHash('sha256').update(contents).digest('hex')}`;
const { SourceMapConsumer } = createRequire(
  require.resolve('terser'),
)('@jridgewell/source-map');
// The standalone optimizer still serves the framework's snapshot benchmark.
// Its isolated implementation tests must not enter the native build path.
const { build } = require('esbuild');
process.chdir(fileURLToPath(new URL('../packages/webui-test-support/', import.meta.url)));

test('standalone example builds prepare the compiler addon and client watches prepare the CLI', async () => {
  for (const app of ['calculator', 'commerce', 'contact-book-manager', 'routes',
    'service-worker', 'streaming', 'todo-webui']) {
    const manifest = new URL(`./app/${app}/package.json`, import.meta.url);
    const { scripts } = JSON.parse(await readFile(manifest, 'utf8'));
    const buildSteps = scripts.build.split(' && ');
    assert.equal(buildSteps[0], 'pnpm run build:deps', app);
    // The E2E and desktop callers can still build a client with prepared dependencies.
    const client = scripts['build:client'];
    assert.match(client, /^node \S*build-client\.mjs$/, app);
    assert.ok(buildSteps.indexOf(
      `node ../../native-build.mjs client ${client.slice('node '.length)}`,
    ) > 0, app);
    assert.deepEqual(scripts['start:client'].split(' && '), [
      'pnpm run build:deps',
      'cargo build --release -p microsoft-webui-cli',
      `${client} --watch`,
    ], app);
  }
});

test('NTP accepts a nonempty static closure covered by native parallel request plans', () => {
  const outputs = {
    'main.js': {
      imports: [
        { path: 'loaded.js', kind: 'import-statement', external: false },
        { path: 'asset.js', kind: 'dynamic-import', external: false },
        { path: 'shared.js', kind: 'dynamic-import', external: false },
        { path: 'nested.js', kind: 'dynamic-import', external: false },
        { path: 'loaded.js', kind: 'dynamic-import', external: false },
      ],
      parallelDynamicImports: { 'asset.js': ['shared.js', 'nested.js', 'loaded.js'] },
    },
    'asset.js': { imports: [
      { path: 'shared.js', kind: 'import-statement', external: false },
      { path: 'loaded.js', kind: 'import-statement', external: false },
    ] },
    'shared.js': { imports: [{ path: 'nested.js', kind: 'import-statement', external: false }] },
    'nested.js': { imports: [] },
    'loaded.js': { imports: [] },
  };
  assert.deepEqual(parallelImportPlans(outputs), [{
    from: 'main.js', target: 'asset.js', dependencies: ['shared.js', 'nested.js', 'loaded.js'],
  }]);
  const missing = structuredClone(outputs);
  missing['main.js'].parallelDynamicImports['asset.js'] = ['shared.js'];
  assert.throws(() => parallelImportPlans(missing), /must request nested\.js/);
  delete missing['main.js'].parallelDynamicImports;
  assert.throws(() => parallelImportPlans(missing), /must request shared\.js/);
  const unrelated = structuredClone(outputs);
  unrelated['main.js'].parallelDynamicImports['asset.js'].push('main.js');
  assert.throws(() => parallelImportPlans(unrelated), /must belong to asset\.js's static closure/);
});

async function sourceFixture(t) {
  const root = await mkdtemp(path.join(packageRoot, '.tmp-client-build-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  await writeFile(path.join(root, 'package.json'), '{"type":"module"}');
  await mkdir(path.join(root, 'src'));
  const source = `/*! @license Fixture MIT */
export function calculate(value) {
  const redundant = value + 1;
  return redundant;
}
export const development = typeof __WEBUI_DEV__ === 'undefined' || __WEBUI_DEV__;
export const diagnostic = ${JSON.stringify('\u2260')};
export class Unicode {
  ${'\u503c'} = ${JSON.stringify('\u96ea')};
  ${'\u65b9\u6cd5'}(suffix) { return this.${'\u503c'} + suffix; }
}
`;
  await writeFile(path.join(root, 'src/index.js'), source);
  return root;
}

async function fixture(t, options = {}) {
  const root = await sourceFixture(t);
  const result = await runWebUIClientBuild({
    project: workspaceRoot,
    entries: { index: path.join(root, 'src/index.js') },
    outDir: path.join(root, 'dist'),
    ...options,
  });
  return { root, result };
}

function outputMetadata(result, root, name = 'index.js') {
  const match = Object.entries(result.metafile.outputs).find(([file]) =>
    path.resolve(workspaceRoot, file) === path.join(root, 'dist', name));
  assert.ok(match, `missing native output metadata for ${name}`);
  return match[1];
}

test('production helper prepares, bundles and finalizes without loading the runtime addon', async t => {
  const root = await sourceFixture(t);
  const { resolve: resolveArtifact } = await import('../packages/webui/dist/platform.js');
  await writeFile(path.join(root, 'src/index.html'), '<!doctype html><html><body><main>{{title}}</main></body></html>');
  const loaded = [];
  const dlopen = process.dlopen;
  t.mock.method(process, 'dlopen', function (...args) {
    loaded.push(args[1]);
    return Reflect.apply(dlopen, this, args);
  });
  const { result, client } = await runWebUIBuild({
    appDir: path.join(root, 'src'),
    plugin: 'webui',
    modulesOut: path.join(root, '.webui'),
    outDir: path.join(root, 'dist'),
    clientOptions: {
      entries: { index: path.join(root, 'src/index.js') },
    },
  });
  assert.ok(result.protocol.length > 0);
  assert.deepEqual(await readFile(path.join(root, 'dist/protocol.bin')), result.protocol);
  assert.ok(Object.keys(client.manifest.outputs).length > 0);
  assert.deepEqual(loaded, [resolveArtifact('build-addon')],
    'preparation and bundling reuse one compiler addon without loading the runtime');
});

test('native production output binds final optimized bytes and retained modules', async t => {
  const { root, result } = await fixture(t);
  assert.deepEqual(result.warnings, []);
  assert.ok(result.files.every(file => file.contents instanceof Uint8Array));
  const code = await readFile(path.join(root, 'dist/index.js'), 'utf8');
  assert.match(code, /@license Fixture MIT/);
  assert.doesNotMatch(code, /redundant/);
  const module = await import(pathToFileURL(path.join(root, 'dist/index.js')).href);
  assert.equal(module.calculate(41), 42);
  assert.equal(module.development, false);
  const manifest = JSON.parse(await readFile(path.join(root, 'dist/webui-projection.json'), 'utf8'));
  const hashes = Object.values(manifest.outputs);
  assert.ok(hashes.includes(hashContent(code)), 'projection must bind the minified bytes');
  const metadata = outputMetadata(result, root);
  assert.equal(metadata.bytes, Buffer.byteLength(code));
  const inputs = await Promise.all(Object.keys(metadata.inputs).filter(path.isAbsolute).map(file => realpath(file)));
  assert.ok(inputs.includes(await realpath(path.join(root, 'src/index.js'))));
  for (const contribution of Object.values(metadata.inputs)) {
    assert.equal(typeof contribution.renderedBytesBeforeMinify, 'number');
    assert.equal('bytesInOutput' in contribution, false);
  }
  assert.equal(metadata.webui, undefined, 'native builds must not invent minifier attribution');
});

test('native build warnings are returned and explicitly reported without failing valid imports', async t => {
  const root = await sourceFixture(t);
  await writeFile(path.join(root, 'src/shared.js'), 'export const value = 42;');
  await writeFile(path.join(root, 'src/index.js'),
    "export { value } from './shared.js'; export const load = () => import('./shared.js');");
  const warnings = [];
  t.mock.method(console, 'warn', (...args) => warnings.push(args.join(' ')));
  const result = await runWebUIClientBuild({
    project: workspaceRoot,
    entries: { index: path.join(root, 'src/index.js') },
    outDir: path.join(root, 'dist'),
  });
  assert.ok(result.warnings.some(warning => warning.code === 'INEFFECTIVE_DYNAMIC_IMPORT'));
  assert.ok(warnings.some(warning => warning.includes('INEFFECTIVE_DYNAMIC_IMPORT')));
  const module = await import(pathToFileURL(path.join(root, 'dist/index.js')).href);
  assert.equal(module.value, 42);
  assert.equal((await module.load()).value, 42);
});

test('minification labels original input attribution without changing the dependency graph', async t => {
  const root = await sourceFixture(t);
  await writeFile(path.join(root, 'src/shared.js'), 'export const shared = value => value + 1;');
  await writeFile(path.join(root, 'src/lazy.js'), `
    import { shared } from './shared.js';
    export const lazy = value => shared(value) * 2;
  `);
  await writeFile(path.join(root, 'src/style.css'), '.fixture { color: red; }');
  await writeFile(path.join(root, 'src/index.js'), `
    import './style.css';
    import { shared } from './shared.js';
    export const calculate = shared;
    export const load = () => import('./lazy.js');
  `);
  let original;
  const result = await build({
    absWorkingDir: root,
    entryPoints: ['src/index.js'],
    outdir: 'dist',
    bundle: true,
    splitting: true,
    format: 'esm',
    sourcemap: 'external',
    metafile: true,
    write: false,
    plugins: [{
      name: 'capture-esbuild-attribution',
      setup(plugin) {
        plugin.onEnd(result => { original = structuredClone(result.metafile); });
      },
    }, minifyClient()],
  });
  assert.ok(original);
  const metafile = JSON.parse(JSON.stringify(result.metafile));
  assert.deepEqual(metafile.inputs, original.inputs);
  const files = new Map(result.outputFiles.map(file => [file.path, file]));
  let javascript = 0;
  let smaller = 0;
  for (const [file, output] of Object.entries(metafile.outputs)) {
    const bytes = files.get(path.resolve(root, file)).contents.byteLength;
    const { bytes: originalBytes, ...originalGraph } = original.outputs[file];
    const { bytes: finalBytes, webui, ...graph } = output;
    assert.equal(finalBytes, bytes);
    assert.deepEqual(graph, originalGraph, `${file} must retain esbuild's graph and attribution`);
    if (file.endsWith('.js')) {
      javascript++;
      if (finalBytes < originalBytes) smaller++;
      assert.deepEqual(webui, {
        inputBytesInOutput: 'before-minification',
        bytesBeforeMinification: originalBytes,
      });
    } else {
      assert.equal(webui, undefined, `${file} was not JavaScript-minified`);
      if (file.endsWith('.css')) assert.equal(finalBytes, originalBytes);
    }
  }
  assert.ok(javascript >= 3, 'entry, dynamic import and shared chunk must all be covered');
  assert.equal(smaller, javascript);
  assert.ok(Object.keys(metafile.outputs).some(file => file.endsWith('.css')));
  assert.ok(Object.keys(metafile.outputs).some(file => file.endsWith('.map')));
});

test('production escapes non-ASCII diagnostics without changing their runtime text', async t => {
  const { root, result } = await fixture(t, { sourcemap: 'hidden' });
  const code = await readFile(path.join(root, 'dist/index.js'), 'utf8');
  assert.doesNotMatch(code, /[^\x00-\x7f]/u);
  assert.match(code, /@license Fixture MIT/);
  const module = await import(pathToFileURL(path.join(root, 'dist/index.js')).href);
  assert.equal(module.diagnostic, '\u2260');
  const instance = new module.Unicode();
  assert.equal(instance['\u503c'], '\u96ea');
  assert.equal(instance['\u65b9\u6cd5']('!'), '\u96ea!');
  assert.equal(instance['\u65b9\u6cd5'].name, '\u65b9\u6cd5');
  assert.equal(instance['\u65b9\u6cd5'].length, 1);
  const manifest = JSON.parse(await readFile(path.join(root, 'dist/webui-projection.json'), 'utf8'));
  assert.ok(Object.values(manifest.outputs).includes(hashContent(code)));
  assert.equal(outputMetadata(result, root).bytes, Buffer.byteLength(code));
  const map = JSON.parse(await readFile(path.join(root, 'dist/index.js.map'), 'utf8'));
  assert.ok(map.sourcesContent.some(source => source.includes('\u503c') && source.includes('\u96ea')));
  const valueIndex = /\\u96ea/i.exec(code)?.index ?? -1;
  assert.ok(valueIndex >= 0);
  const position = code.slice(0, valueIndex).split('\n');
  const original = new SourceMapConsumer(map).originalPositionFor({
    line: position.length, column: position.at(-1).length,
  });
  assert.ok(original.source.endsWith('src/index.js'));
  assert.equal(original.line, 9);
});

for (const sourcemap of ['none', 'file', 'hidden', 'inline']) {
  test(`in-memory output preserves exports, hashes and source maps (${sourcemap})`, async t => {
    const { root, result } = await fixture(t, { write: false, sourcemap });
    const js = result.files.find(file => file.path.endsWith('.js'));
    const text = new TextDecoder().decode(js.contents);
    assert.ok(Object.values(result.manifest.outputs).includes(hashContent(js.contents)));
    await assert.rejects(readFile(path.join(root, 'dist/index.js')), { code: 'ENOENT' });
    await assert.rejects(readFile(result.manifestPath), { code: 'ENOENT' });
    const module = await import(`data:text/javascript;base64,${Buffer.from(js.contents).toString('base64')}`);
    assert.equal(module.calculate(5), 6);
    const external = result.files.find(file => file.path.endsWith('.map'));
    const inline = /sourceMappingURL=data:application\/json[^,]*,([^\s]+)/.exec(text);
    if (sourcemap === 'none') {
      assert.equal(external, undefined);
      assert.doesNotMatch(text, /sourceMappingURL/);
      return;
    }
    const map = JSON.parse(external
      ? new TextDecoder().decode(external.contents)
      : Buffer.from(inline[1], 'base64').toString());
    assert.ok(map.sources.some(source => source.endsWith('src/index.js')));
    assert.ok(map.sourcesContent.some(source => source.includes('redundant')));
    assert.ok(map.mappings.length);
    const position = text.slice(0, text.indexOf('return')).split('\n');
    const original = new SourceMapConsumer(map).originalPositionFor({
      line: position.length,
      column: position.at(-1).length,
    });
    assert.ok(original.source.endsWith('src/index.js'));
    assert.ok(original.line >= 2 && original.line <= 4);
    if (sourcemap === 'hidden') assert.doesNotMatch(text, /sourceMappingURL/);
    if (sourcemap === 'file') assert.match(text, /sourceMappingURL=index\.js\.map/);
    if (sourcemap === 'inline') assert.ok(inline);
  });
}

test('explicit minify false and development policy remain independent', async t => {
  const { root, result } = await fixture(t, {
    minify: false,
    development: true,
  });
  const code = await readFile(path.join(root, 'dist/index.js'), 'utf8');
  assert.match(code, /function calculate\(value\) \{\n/);
  const module = await import(pathToFileURL(path.join(root, 'dist/index.js')).href);
  assert.equal(module.development, true);
  assert.equal(outputMetadata(result, root).webui, undefined);
});

test('hashed output names change when final minifier settings change', async t => {
  const options = { entryNames: '[name]-[hash].js', write: false };
  const first = await fixture(t, { ...options, minify: false });
  const second = await fixture(t, options);
  const named = first.result.files.find(file => file.path.endsWith('.js'));
  const mangled = second.result.files.find(file => file.path.endsWith('.js'));
  assert.notDeepEqual(named.contents, mangled.contents);
  assert.notEqual(path.basename(named.path), path.basename(mangled.path));
});

test('CommonJS, custom extensions and preserved function names remain supported', async t => {
  const root = await sourceFixture(t);
  const result = await build({
    absWorkingDir: root, entryPoints: ['src/index.js'], outdir: 'dist',
    bundle: true, metafile: true, plugins: [minifyClient()],
    format: 'cjs',
    splitting: false,
    keepNames: true,
    target: 'es2015',
    outExtension: { '.js': '.cjs' },
    sourcemap: 'external',
    write: false,
  });
  const js = result.outputFiles.find(file => file.path.endsWith('.cjs'));
  const module = { exports: {} };
  runInNewContext(js.text, { module, exports: module.exports });
  assert.equal(module.exports.calculate(41), 42);
  assert.equal(module.exports.calculate.name, 'calculate');
  assert.equal(result.metafile.outputs['dist/index.cjs'].bytes, js.contents.length);
  assert.equal(
    result.metafile.outputs['dist/index.cjs'].webui.inputBytesInOutput,
    'before-minification',
  );
  assert.ok(result.outputFiles.some(file => file.path.endsWith('.cjs.map')));
});

test('custom ESM extensions retain minification attribution', async t => {
  const root = await sourceFixture(t);
  const result = await build({
    absWorkingDir: root, entryPoints: ['src/index.js'], outdir: 'dist',
    bundle: true, format: 'esm', metafile: true, plugins: [minifyClient()],
    outExtension: { '.js': '.mjs' },
    write: false,
  });
  const js = result.outputFiles.find(file => file.path.endsWith('.mjs'));
  const output = result.metafile.outputs['dist/index.mjs'];
  assert.equal(output.bytes, js.contents.length);
  assert.equal(output.webui.inputBytesInOutput, 'before-minification');
  assert.ok(output.webui.bytesBeforeMinification > output.bytes);
});

async function publishedWatchOutput(root, watcher, previous) {
  const output = path.join(root, 'dist/index.js');
  const manifestPath = path.join(root, 'dist/webui-projection.json');
  for (let attempt = 0; attempt < 200; attempt++) {
    if (watcher.process.exitCode !== null) {
      throw new Error(`Native watcher exited before publishing (${watcher.process.exitCode}).`);
    }
    try {
      const code = await readFile(output, 'utf8');
      const proof = await readFile(manifestPath, 'utf8');
      const manifest = JSON.parse(proof);
      if (code !== previous && Object.values(manifest.outputs).includes(hashContent(code))) {
        return { code, proof, manifest };
      }
    } catch (error) {
      if (error.code !== 'ENOENT' && !(error instanceof SyntaxError)) throw error;
    }
    await delay(50);
  }
  throw new Error('Native watcher did not publish new output and matching proof.');
}

function nativeWatchFailure(watcher) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      watcher.process.stderr.off('data', observe);
      reject(new Error('Native watcher did not report the invalid source.'));
    }, 10_000);
    const observe = bytes => {
      if (!bytes.toString().includes('native browser build failed')) return;
      clearTimeout(timer);
      watcher.process.stderr.off('data', observe);
      resolve();
    };
    watcher.process.stderr.on('data', observe);
  });
}

test('native watch rebuilds real edits, retains development output and recovers after errors',
  { timeout: 20_000 }, async t => {
  const original = process.argv;
  process.argv = [...original, '--watch'];
  let context;
  try {
    const built = await fixture(t);
    context = built.result;
    const output = path.join(built.root, 'dist/index.js');
    const source = path.join(built.root, 'src/index.js');
    const { code } = await publishedWatchOutput(built.root, context);
    assert.match(code, /function calculate\(value\) \{\n/);
    const module = await import(pathToFileURL(path.join(built.root, 'dist/index.js')).href);
    assert.equal(module.development, true);
    await writeFile(source, 'export const calculate = value => value + 2; export const development = __WEBUI_DEV__;');
    const rewritten = await publishedWatchOutput(built.root, context, code);
    const updated = await import(`${pathToFileURL(output).href}?edited`);
    assert.equal(updated.calculate(40), 42);
    assert.equal(updated.development, true);

    const failed = nativeWatchFailure(context);
    await writeFile(source, 'export const = ;');
    await failed;
    assert.equal(await readFile(output, 'utf8'), rewritten.code, 'failed edits preserve the last good output');
    assert.equal(await readFile(path.join(built.root, 'dist/webui-projection.json'), 'utf8'), rewritten.proof);

    await writeFile(source, 'export const calculate = value => value + 3;');
    const recovered = await publishedWatchOutput(built.root, context, rewritten.code);
    const repaired = await import(`${pathToFileURL(output).href}?repaired`);
    assert.equal(repaired.calculate(39), 42);
    assert.ok(Object.values(recovered.manifest.outputs).includes(hashContent(await readFile(output))));
  } finally {
    process.argv = original;
    await context?.dispose();
  }
});

test('production emits no orphan hydration diagnostic chunks', async t => {
  const framework = fileURLToPath(new URL('../packages/webui-framework/src/index.ts', import.meta.url));
  const entry = '\0fixture:framework-entry';
  const { result } = await fixture(t, {
    entries: { index: entry },
    virtualModules: { [entry]: `export { WebUIElement } from ${JSON.stringify(framework.replaceAll('\\', '/'))};` },
    write: false,
  });
  for (const [file, output] of Object.entries(result.metafile.outputs)) {
    assert.doesNotMatch(file, /hydration-mismatch/);
    assert.ok(!Object.keys(output.inputs).some(input => input.endsWith('hydration-mismatch.ts')));
  }
});

test('native split entries preserve shared code, lazy imports and CSS import attributes', async t => {
  const root = await sourceFixture(t);
  await writeFile(path.join(root, 'src/shared.js'), 'export const shared = value => value + 1;');
  await writeFile(path.join(root, 'src/lazy.js'),
    "import { shared } from './shared.js'; export const lazy = value => shared(value) * 2;");
  await writeFile(path.join(root, 'src/island.js'),
    "import { shared } from './shared.js'; globalThis.island = shared(3);");
  await writeFile(path.join(root, 'src/style.css'), '.fixture::after { content: "雪"; }');
  await writeFile(path.join(root, 'src/index.js'), `
    import sheet from './style.css' with { type: 'css' };
    import { shared } from './shared.js';
    export { sheet, shared };
    export const load = () => import('./lazy.js');
  `);
  const result = await runWebUIClientBuild({
    project: workspaceRoot,
    entries: { index: path.join(root, 'src/index.js'), island: path.join(root, 'src/island.js') },
    outDir: path.join(root, 'dist'),
  });
  const outputs = Object.values(result.metafile.outputs);
  const entryPoints = await Promise.all(outputs.flatMap(output =>
    output.entryPoint ? [realpath(output.entryPoint)] : []));
  assert.ok(entryPoints.includes(await realpath(path.join(root, 'src/index.js'))));
  assert.ok(entryPoints.includes(await realpath(path.join(root, 'src/island.js'))));
  assert.ok(outputs.some(output => output.imports.some(edge =>
    edge.kind === 'dynamic-import' && !edge.external)));
  assert.ok(outputs.some(output => output.imports.some(edge =>
    edge.kind === 'import-statement' && !edge.external)));
  for (const [file, output] of Object.entries(result.metafile.outputs)) {
    const bytes = await readFile(path.resolve(workspaceRoot, file));
    assert.equal(output.bytes, bytes.length);
    assert.equal(output.webui, undefined);
  }
  const priorSheet = globalThis.CSSStyleSheet;
  globalThis.CSSStyleSheet = class {
    replaceSync(text) { this.text = text; }
  };
  t.after(() => {
    if (priorSheet === undefined) delete globalThis.CSSStyleSheet;
    else globalThis.CSSStyleSheet = priorSheet;
  });
  const module = await import(pathToFileURL(path.join(root, 'dist/index.js')).href);
  assert.equal(module.shared(41), 42);
  assert.match(module.sheet.text, /雪/);
  assert.equal((await module.load()).lazy(20), 42);
  const manifest = JSON.parse(await readFile(result.manifestPath, 'utf8'));
  assert.deepEqual(manifest, result.manifest);
  assert.equal(manifest.adapter.name, 'rolldown');
});
