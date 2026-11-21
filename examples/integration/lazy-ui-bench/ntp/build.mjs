// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Builds one NTP fixture the way an application with its own bundler would:
// Node `prepareBuild()` retains generated modules and compilation options;
// native Rolldown bundles the client and proves its state before finalization.

import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import { brotliCompressSync, constants } from 'node:zlib';
import { ASSET_ROOTS, DEFERRED, STATE, UIS, benchConfig, regionsOf, uiEntries, writeApp, writeMain } from './app.mjs';

const TYPES = { '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json' };

export const brotli = (buffer, quality = 11) =>
  brotliCompressSync(buffer, { params: { [constants.BROTLI_PARAM_QUALITY]: quality } });

const compress = buffer => ({ raw: buffer, br: brotli(buffer) });

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

/** Validate native request plans without erasing the bundle's static graph. */
export function parallelImportPlans(outputs, label = 'NTP') {
  const groups = [];
  for (const [from, output] of Object.entries(outputs)) {
    const available = new Set(staticClosure(outputs, from));
    const plans = new Map();
    const nativePlans = output.parallelDynamicImports ?? {};
    assert.ok(typeof nativePlans === 'object' && nativePlans !== null && !Array.isArray(nativePlans),
      `${label}: native parallel plans must map targets to dependencies`);
    for (const [target, dependencies] of Object.entries(nativePlans)) {
      assert.ok(output.imports.some(edge =>
        !edge.external && edge.kind === 'dynamic-import' && edge.path === target),
      `${label}: parallel plan must name a dynamic target of ${from}`);
      const closure = new Set(staticClosure(outputs, target));
      closure.delete(target);
      assert.ok(Array.isArray(dependencies), `${label}: parallel plan must list dependencies`);
      for (const dependency of dependencies) {
        assert.ok(outputs[dependency] && closure.has(dependency),
          `${label}: parallel dependency ${dependency} must belong to ${target}'s static closure`);
        assert.ok(output.imports.some(edge =>
          !edge.external && edge.kind === 'dynamic-import' && edge.path === dependency),
        `${label}: native metadata must record the injected import of ${dependency}`);
      }
      plans.set(target, new Set(dependencies));
      groups.push({ from, target, dependencies: [...dependencies] });
    }
    for (const edge of output.imports) {
      if (edge.external || edge.kind !== 'dynamic-import') continue;
      assert.ok(outputs[edge.path], `${label}: native dynamic target ${edge.path} must exist`);
      const needed = staticClosure(outputs, edge.path)
        .filter(dependency => dependency !== edge.path && !available.has(dependency));
      // Injected import() calls are part of their original target's group, not
      // independent authored calls requiring another synthetic plan.
      const covering = plans.has(edge.path)
        ? [plans.get(edge.path)]
        : [...plans.values()].filter(dependencies => dependencies.has(edge.path));
      for (const dependency of needed) {
        assert.ok(covering.some(dependencies => dependencies.has(dependency)),
          `${label}: native parallel plan for ${edge.path} must request ${dependency}`);
      }
    }
  }
  return groups;
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
      raw += JSON.stringify(json.templates[id]).length
        + JSON.stringify(json.componentStyles?.resources?.[id] ?? '').length;
      delete json.templates[id];
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
export async function buildFixture({ key, scratch, root, prepareBuild, Protocol, bundleClient, probeTriggers }) {
  const [name, css] = key.split('-');
  const variant = name === 'probe' ? 'lazy' : name;
  const dir = join(scratch, key);
  writeApp(dir, variant, name === 'probe' ? probeTriggers : undefined);
  const modulesOut = join(dir, '.webui');
  const options = {
    appDir: dir, plugin: 'webui', css, modulesOut,
    ...(variant === 'assets' ? { componentAssetRoots: ASSET_ROOTS } : {}),
  };
  const prepared = prepareBuild({ ...options, metafile: true });
  const first = {
    lazyScriptsModule: prepared.lazyScriptsModule,
    componentAssetFiles: prepared.componentAssetFiles,
    lazyPackFiles: prepared.lazyPackFiles,
  };
  prepared.writeModules();
  const regions = variant === 'lazy' ? regionsOf(first.lazyPackFiles) : {};
  if (variant === 'lazy') assert.equal(Object.keys(regions).length, Object.keys(UIS).length, `${key}: every UI is a lazy region`);
  const config = benchConfig(variant, regions);
  writeMain(dir, variant, config);
  const outdir = join(dir, 'dist');
  const manifest = join(dir, 'webui-projection.json');
  const tsconfig = join(dir, 'tsconfig.json');
  writeFileSync(tsconfig, JSON.stringify({
    compilerOptions: { experimentalDecorators: true, useDefineForClassFields: false },
  }));
  const result = await bundleClient({
    project: root, entries: { main: join(dir, 'main.ts') }, outDir: outdir,
    format: 'esm', minify: true, target: 'es2022', tsconfig,
    framework: join(root, 'packages', 'webui-framework'),
    parallelDynamicImports: true,
    entryNames: '[name].js', chunkNames: 'chunk-[hash].js', manifestPath: manifest,
  });
  for (const warning of result.warnings) console.warn(`[${key}] ${warning.code}: ${warning.message}`);
  const built = prepared.finish({ projectionManifests: [manifest] });
  if (variant === 'lazy') {
    assert.deepEqual(built.lazyScriptsModule, first.lazyScriptsModule, `${key}: lazy.webui.js is stable across preparation and finalization`);
  }
  if (variant === 'assets') {
    assert.deepEqual(built.componentAssetFiles, first.componentAssetFiles, `${key}: component assets are stable across preparation and finalization`);
  }
  assert.equal(built.lazyPackFiles.length, variant === 'lazy' ? Object.keys(UIS).length : 0, `${key}: pack count`);

  const files = new Map();
  const outputs = result.metafile.outputs;
  const served = outputKey => relative(outdir, resolve(root, outputKey)).replaceAll('\\', '/');
  for (const outputKey of Object.keys(outputs)) {
    if (!outputKey.endsWith('.js')) continue;
    files.set(served(outputKey), { type: TYPES['.js'], body: compress(readFileSync(resolve(root, outputKey))) });
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
  for (const [outputKey, output] of Object.entries(outputs)) {
    for (const input of Object.keys(output.inputs)) {
      entries.set(resolve(root, input), outputKey);
    }
  }
  for (const [outputKey, output] of Object.entries(outputs)) {
    if (output.entryPoint) entries.set(resolve(root, output.entryPoint), outputKey);
  }
  const mainOutput = entries.get(join(dir, 'main.ts'));
  assert.ok(mainOutput, `${key}: native bundle must include the startup entry`);
  const startupOutputs = new Set(staticClosure(outputs, mainOutput));
  const parallelImports = parallelImportPlans(outputs, key).map(group => ({
    from: served(group.from), target: served(group.target), dependencies: group.dependencies.map(served),
  }));
  const startupScripts = new Set([...startupOutputs].map(served));
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
      const outputEntry = entries.get(join(dir, entry));
      assert.ok(outputEntry, `${key}: native bundle must retain UI entry ${entry}`);
      for (const output of staticClosure(outputs, outputEntry)) {
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
    key, variant, css, dir, protocol, files, packs, regions, config, ui, startupScripts, startupCss, parallelImports,
    warnings: built.warnings,
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
