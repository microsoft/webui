// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { test } from 'node:test';
import { runInNewContext } from 'node:vm';
import { runWebUIClientBuild } from './build-client.mjs';

const packageRoot = fileURLToPath(new URL('../packages/webui/', import.meta.url));
const require = createRequire(path.join(packageRoot, 'package.json'));
const { hashContent } = await import(
  pathToFileURL(path.join(packageRoot, 'dist/projection/index.js')).href
);
const { SourceMapConsumer } = createRequire(
  require.resolve('terser'),
)('@jridgewell/source-map');
process.chdir(fileURLToPath(new URL('../packages/webui-test-support/', import.meta.url)));

async function fixture(t, options = {}) {
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
  const result = await runWebUIClientBuild({
    absWorkingDir: root,
    entryPoints: ['src/index.js'],
    outdir: 'dist',
    ...options,
  });
  return { root, result };
}

test('production output is optimized before projection hashes and metafile sizes', async t => {
  const { root, result } = await fixture(t);
  assert.equal(result.outputFiles, undefined);
  const code = await readFile(path.join(root, 'dist/index.js'), 'utf8');
  assert.match(code, /@license Fixture MIT/);
  assert.doesNotMatch(code, /redundant/);
  const module = await import(pathToFileURL(path.join(root, 'dist/index.js')).href);
  assert.equal(module.calculate(41), 42);
  assert.equal(module.development, false);
  const manifest = JSON.parse(await readFile(path.join(root, 'dist/webui-projection.json'), 'utf8'));
  const hashes = Object.values(manifest.outputs);
  assert.ok(hashes.includes(hashContent(code)), 'projection must bind the minified bytes');
  assert.equal(result.metafile.outputs['dist/index.js'].bytes, Buffer.byteLength(code));
});

test('production escapes non-ASCII diagnostics without changing their runtime text', async t => {
  const { root, result } = await fixture(t, { sourcemap: 'external' });
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
  assert.equal(result.metafile.outputs['dist/index.js'].bytes, Buffer.byteLength(code));
  const map = JSON.parse(await readFile(path.join(root, 'dist/index.js.map'), 'utf8'));
  assert.ok(map.sourcesContent.some(source => source.includes('\u503c') && source.includes('\u96ea')));
  const valueIndex = code.indexOf('\\u96ea');
  assert.ok(valueIndex >= 0);
  const position = code.slice(0, valueIndex).split('\n');
  const original = new SourceMapConsumer(map).originalPositionFor({
    line: position.length, column: position.at(-1).length,
  });
  assert.ok(original.source.endsWith('src/index.js'));
  assert.equal(original.line, 9);
});

for (const sourcemap of [false, true, 'external', 'inline', 'both']) {
  test(`in-memory output preserves exports, hashes and source maps (${sourcemap})`, async t => {
    const { result } = await fixture(t, { write: false, sourcemap });
    const js = result.outputFiles.find(file => file.path.endsWith('.js'));
    assert.equal(js.text, new TextDecoder().decode(js.contents));
    assert.ok(js.hash);
    const module = await import(`data:text/javascript;base64,${Buffer.from(js.contents).toString('base64')}`);
    assert.equal(module.calculate(5), 6);
    if (!sourcemap) return;
    const external = result.outputFiles.find(file => file.path.endsWith('.map'));
    const inline = /sourceMappingURL=data:application\/json[^,]*,([^\s]+)/.exec(js.text);
    const map = JSON.parse(external?.text ?? Buffer.from(inline[1], 'base64').toString());
    assert.ok(map.sources.some(source => source.endsWith('src/index.js')));
    assert.ok(map.sourcesContent.some(source => source.includes('redundant')));
    assert.ok(map.mappings.length);
    const position = js.text.slice(0, js.text.indexOf('return')).split('\n');
    const original = new SourceMapConsumer(map).originalPositionFor({
      line: position.length,
      column: position.at(-1).length,
    });
    assert.ok(original.source.endsWith('src/index.js'));
    assert.ok(original.line >= 2 && original.line <= 4);
    if (sourcemap === 'external') assert.doesNotMatch(js.text, /sourceMappingURL/);
    if (sourcemap === 'inline' || sourcemap === 'both') assert.ok(inline);
  });
}

test('explicit minify false and developer flag overrides remain supported', async t => {
  const { root } = await fixture(t, {
    minify: false,
    define: { __WEBUI_DEV__: 'true' },
  });
  const code = await readFile(path.join(root, 'dist/index.js'), 'utf8');
  assert.match(code, /redundant/);
  const module = await import(pathToFileURL(path.join(root, 'dist/index.js')).href);
  assert.equal(module.development, true);
});

test('hashed output names change when final minifier settings change', async t => {
  const options = { entryNames: '[name]-[hash]', write: false };
  const first = await fixture(t, { ...options, minifyIdentifiers: false });
  const second = await fixture(t, options);
  const named = first.result.outputFiles.find(file => file.path.endsWith('.js'));
  const mangled = second.result.outputFiles.find(file => file.path.endsWith('.js'));
  assert.notEqual(named.text, mangled.text);
  assert.notEqual(path.basename(named.path), path.basename(mangled.path));
});

test('CommonJS, custom extensions and preserved function names remain supported', async t => {
  const { result } = await fixture(t, {
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
  assert.ok(result.outputFiles.some(file => file.path.endsWith('.cjs.map')));
});

test('watch mode retains development diagnostics and readable output', async t => {
  const original = process.argv;
  process.argv = [...original, '--watch'];
  let context;
  try {
    const built = await fixture(t);
    context = built.result;
    await context.rebuild();
    const code = await readFile(path.join(built.root, 'dist/index.js'), 'utf8');
    assert.match(code, /redundant/);
    const module = await import(pathToFileURL(path.join(built.root, 'dist/index.js')).href);
    assert.equal(module.development, true);
  } finally {
    process.argv = original;
    await context?.dispose();
  }
});

test('production emits no orphan hydration diagnostic chunks', async t => {
  const framework = fileURLToPath(new URL('../packages/webui-framework/src/index.ts', import.meta.url));
  const { result } = await fixture(t, {
    stdin: {
      contents: `export { WebUIElement } from ${JSON.stringify(framework.replaceAll('\\', '/'))};`,
      resolveDir: packageRoot,
      sourcefile: 'framework-entry.ts',
      loader: 'ts',
    },
    entryPoints: [],
    write: false,
  });
  for (const [file, output] of Object.entries(result.metafile.outputs)) {
    assert.doesNotMatch(file, /hydration-mismatch/);
    assert.ok(!Object.keys(output.inputs).some(input => input.endsWith('hydration-mismatch.ts')));
  }
});
