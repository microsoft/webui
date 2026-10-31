// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Builds one NTP fixture the way an application with its own bundler would:
// `webui build` (Node `build()`) writes the generated modules, esbuild bundles
// the client with code splitting and the projection plugin, then a second
// build with the projection manifest produces the protocol, CSS, and packs.

import assert from 'node:assert/strict';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { brotliCompressSync, constants } from 'node:zlib';
import { ASSET_ROOTS, DEFERRED, STATE, UIS, benchConfig, regionsOf, uiEntries, writeApp, writeMain } from './app.mjs';

const TYPES = { '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json' };

export const brotli = (buffer, quality = 11) =>
  brotliCompressSync(buffer, { params: { [constants.BROTLI_PARAM_QUALITY]: quality } });

const compress = buffer => ({ raw: buffer, br: brotli(buffer) });

/**
 * esbuild leaves a split chunk's shared dependencies as static imports that the
 * browser discovers only after the chunk arrives. webpack and rspack load an
 * async chunk group in parallel, so this rewrites each dynamic import to also
 * import the target's static dependency closure at the same time. Modules
 * already loaded resolve from the module map without a request.
 */
function parallelChunkGroups() {
  return {
    name: 'ntp-parallel-chunk-groups',
    setup(build) {
      build.onEnd(result => {
        if (result.errors.length > 0 || !result.outputFiles || !result.metafile) return;
        const cwd = build.initialOptions.absWorkingDir;
        const outputs = result.metafile.outputs;
        const keys = new Map(Object.keys(outputs).map(key => [resolve(cwd, key), key]));
        for (let i = 0; i < result.outputFiles.length; i++) {
          const file = result.outputFiles[i];
          if (!file.path.endsWith('.js')) continue;
          const from = dirname(file.path);
          const text = file.text;
          const rewritten = text.replace(/import\((["'])(\.{1,2}\/[^"']+\.js)\1\)/g, (match, quote, specifier) => {
            const key = keys.get(resolve(from, specifier));
            const dependencies = key ? staticClosure(outputs, key).filter(dependency => dependency !== key) : [];
            if (dependencies.length === 0) return match;
            const specifiers = [specifier, ...dependencies.map(dependency =>
              './' + relative(from, resolve(cwd, dependency)).replaceAll('\\', '/'))];
            return `Promise.all([${specifiers.map(entry => `import(${quote}${entry}${quote})`).join(',')}]).then(m=>m[0])`;
          });
          if (rewritten === text) continue;
          const contents = Buffer.from(rewritten);
          result.outputFiles[i] = { path: file.path, contents, hash: file.hash, get text() { return rewritten; } };
        }
      });
    },
  };
}

/** An output and every output it reaches through static imports. */
export function staticClosure(outputs, start) {
  const reached = new Set([start]);
  const pending = [start];
  while (pending.length > 0) {
    for (const edge of outputs[pending.pop()]?.imports ?? []) {
      if (edge.kind !== 'import-statement' || edge.external || reached.has(edge.path)) continue;
      reached.add(edge.path);
      pending.push(edge.path);
    }
  }
  return [...reached];
}

function writeModules(dir, files) {
  for (let i = 0; i + 1 < files.length; i += 2) {
    mkdirSync(dirname(join(dir, files[i])), { recursive: true });
    writeFileSync(join(dir, files[i]), files[i + 1]);
  }
}

/** Bytes a later pack repeats from an earlier one, and the Brotli bytes they cost. */
export function duplication(packs) {
  const owners = new Map();
  for (const pack of packs) {
    for (const id of Object.keys(pack.json.templates ?? {})) {
      if (id !== pack.region) owners.set(id, (owners.get(id) ?? 0) + 1);
    }
  }
  const shared = [...owners].filter(([, count]) => count > 1).map(([id]) => id);
  let raw = 0;
  const seen = new Set();
  let stripped = 0;
  let total = 0;
  for (const pack of packs) {
    const json = structuredClone(pack.json);
    for (const id of shared) {
      if (!json.templates?.[id]) continue;
      if (!seen.has(id)) {
        seen.add(id);
        continue;
      }
      raw += JSON.stringify(json.templates[id]).length + (json.templateFunctions?.[id]?.length ?? 0)
        + JSON.stringify(json.componentStyles?.resources?.[id] ?? '').length;
      delete json.templates[id];
      if (json.templateFunctions) delete json.templateFunctions[id];
      if (json.componentStyles?.resources) delete json.componentStyles.resources[id];
      if (json.componentStyles?.closures) delete json.componentStyles.closures[id];
    }
    total += brotli(Buffer.from(pack.content)).length;
    stripped += brotli(Buffer.from(JSON.stringify(json))).length;
  }
  return { templates: shared, raw, brotli: total - stripped };
}

const hrefs = text => [...text.matchAll(/"href":"([^"]+)"/g)].map(match => match[1]);

/**
 * Build fixture `key` (`<variant>-<css>`, or `probe-<css>` for the lazy app
 * with eager and idle triggers) under `scratch`.
 */
export async function buildFixture({ key, scratch, root, build, Protocol, bundle, esbuildProjection, probeTriggers }) {
  const [name, css] = key.split('-');
  const variant = name === 'probe' ? 'lazy' : name;
  const dir = join(scratch, key);
  writeApp(dir, variant, name === 'probe' ? probeTriggers : undefined);
  const modulesOut = join(dir, '.webui');
  const options = {
    appDir: dir, plugin: 'webui', css, modulesOut,
    ...(variant === 'assets' ? { componentAssetRoots: ASSET_ROOTS } : {}),
  };
  let first;
  if (variant !== 'eager') {
    // The generated modules never depend on the projection, so they come first.
    first = build(options);
    if (variant === 'lazy') {
      const { fileName, content, declaration } = first.lazyScriptsModule;
      writeModules(modulesOut, [fileName, content, declaration.fileName, declaration.content]);
    } else writeModules(modulesOut, first.componentAssetFiles);
  }
  const regions = variant === 'lazy' ? regionsOf(first.lazyPackFiles) : {};
  if (variant === 'lazy') assert.equal(Object.keys(regions).length, Object.keys(UIS).length, `${key}: every UI is a lazy region`);
  const config = benchConfig(variant, regions);
  writeMain(dir, variant, config);
  const outdir = join(dir, 'dist');
  const manifest = join(dir, 'webui-projection.json');
  const result = await bundle({
    entryPoints: [join(dir, 'main.ts')], outdir, bundle: true, format: 'esm', splitting: true, minify: true,
    platform: 'browser', target: 'es2022', legalComments: 'none', absWorkingDir: dir,
    entryNames: '[name]', chunkNames: 'chunk-[hash]', write: false, metafile: true,
    nodePaths: [join(root, 'examples', 'app', 'lazy-ui', 'node_modules')],
    tsconfigRaw: { compilerOptions: { experimentalDecorators: true, useDefineForClassFields: false } },
    supported: { 'import-attributes': true }, define: { __WEBUI_DEV__: 'false' },
    plugins: [parallelChunkGroups(), esbuildProjection({ manifest })], logLevel: 'silent',
  }).catch(error => {
    const notes = (error.errors ?? []).flatMap(entry => [entry.text, ...(entry.notes ?? []).map(note => note.text)]);
    throw new Error(`${key}: ${error.message}\n${notes.join('\n')}`);
  });
  for (const file of result.outputFiles) {
    mkdirSync(dirname(file.path), { recursive: true });
    writeFileSync(file.path, file.contents);
  }
  const built = build({ ...options, projectionManifests: [manifest], metafile: true });
  if (variant === 'lazy') {
    assert.deepEqual(built.lazyScriptsModule, first.lazyScriptsModule, `${key}: lazy.webui.js is stable across passes`);
  }
  if (variant === 'assets') {
    assert.deepEqual(built.componentAssetFiles, first.componentAssetFiles, `${key}: component assets are stable across passes`);
  }
  assert.equal(built.lazyPackFiles.length, variant === 'lazy' ? Object.keys(UIS).length : 0, `${key}: pack count`);

  const files = new Map();
  const outputs = result.metafile.outputs;
  const served = outputKey => relative(outdir, resolve(dir, outputKey)).replaceAll('\\', '/');
  for (const outputKey of Object.keys(outputs)) {
    if (!outputKey.endsWith('.js')) continue;
    files.set(served(outputKey), { type: TYPES['.js'], body: compress(readFileSync(resolve(dir, outputKey))) });
  }
  for (let i = 0; i + 1 < built.cssFiles.length; i += 2) {
    files.set(built.cssFiles[i], { type: TYPES['.css'], body: compress(Buffer.from(built.cssFiles[i + 1])) });
  }
  const packs = [];
  for (const pack of built.lazyPackFiles) {
    const body = compress(Buffer.from(pack.content));
    files.set(pack.path, { type: TYPES['.json'], body });
    packs.push({ ...pack, json: JSON.parse(pack.content), brotli: body.br.length });
  }

  const entries = new Map();
  for (const [outputKey, output] of Object.entries(outputs)) if (output.entryPoint) entries.set(output.entryPoint, outputKey);
  const startupScripts = new Set(staticClosure(outputs, entries.get('main.ts')).map(served));
  const startupCss = new Set();
  for (let i = 0; i < built.cssFiles.length; i += 2) {
    if (!DEFERRED.some(tag => built.cssFiles[i].startsWith(tag))) startupCss.add(built.cssFiles[i]);
  }
  const brSize = path => files.get(path)?.body.br.length ?? 0;
  const assetText = variant === 'assets' ? first.componentAssetFiles.filter((_, i) => i % 2 === 1).join('\n') : '';
  const ui = {};
  for (const id of Object.keys(UIS)) {
    const chunks = new Set();
    for (const entry of uiEntries(id, variant)) {
      for (const output of staticClosure(outputs, entries.get(entry))) {
        if (!startupScripts.has(served(output))) chunks.add(served(output));
      }
    }
    const pack = packs.find(entry => entry.region === regions[id]);
    // Packs report the stylesheet files a link build fetches for them; component assets name them in their modules.
    const styles = new Set(css === 'link' && UIS[id].kind === 'dialog'
      ? (pack ? pack.stylesheets : UIS[id].scripts.flatMap(tag => hrefs(assetText).filter(href => href.startsWith(tag))))
      : []);
    ui[id] = {
      pack: pack?.brotli ?? 0,
      chunks: [...chunks],
      chunkBytes: [...chunks].reduce((sum, path) => sum + brSize(path), 0),
      css: [...styles],
      cssBytes: [...styles].reduce((sum, path) => sum + brSize(path), 0),
      scripts: pack?.scripts ?? [],
    };
  }
  const protocol = new Protocol(built.protocol, { plugin: 'webui' });
  const html = Buffer.from(protocol.render(STATE));
  return {
    key, variant, css, dir, protocol, files, packs, regions, config, ui, startupScripts, startupCss,
    warnings: [...(first?.warnings ?? []), ...built.warnings],
    lazyScriptsModule: first?.lazyScriptsModule?.content,
    sizes: {
      htmlBrotli5: brotli(html, 5).length,
      startupScriptsBrotli: [...startupScripts].reduce((sum, path) => sum + brSize(path), 0),
      startupScripts: startupScripts.size,
      startupCssBrotli: [...startupCss].reduce((sum, path) => sum + brSize(path), 0),
      startupCss: startupCss.size,
      packsBrotli: packs.reduce((sum, pack) => sum + pack.brotli, 0),
      duplicated: packs.length ? duplication(packs) : { templates: [], raw: 0, brotli: 0 },
    },
  };
}
