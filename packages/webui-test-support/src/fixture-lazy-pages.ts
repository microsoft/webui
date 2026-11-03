// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { build, Protocol } from '@microsoft/webui';
import { build as bundle } from 'esbuild';
import { esbuildProjection } from '@microsoft/webui/projection.js';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { relative, resolve, sep } from 'node:path';
import { brotliCompressSync, gzipSync } from 'node:zlib';
import type { FixtureRequestContext } from './fixture-server.js';

/** Generated lazy graph with manual page switching and real partial route navigation. */
export async function prepareLazyPages(
  fixture: string,
  outDir: string,
  tsconfig: string,
): Promise<(context: FixtureRequestContext) => boolean> {
  const generatedDir = resolve(outDir, 'generated');
  mkdirSync(generatedDir, { recursive: true });
  const options = {
    appDir: resolve(fixture, 'src'), plugin: 'webui', css: 'link' as const,
    modulesOut: generatedDir, assetBase: '/lazy-pages/', entry: 'page.html',
  };
  const first = build(options);
  if (!first.lazyScriptsModule) throw new Error('Lazy pages fixture must generate its installer.');
  for (const file of [first.lazyScriptsModule, first.lazyScriptsModule.declaration]) {
    writeFileSync(resolve(generatedDir, file.fileName), file.content);
  }
  const manifest = resolve(outDir, 'webui-projection.json');
  const publicPath = '/dist/lazy-pages/';
  const output = await bundle({
    entryPoints: {
      application: resolve(fixture, 'application.ts'),
      routed: resolve(fixture, '../../../../webui-delivery/tests/fixtures/lazy-routes/index.ts'),
    },
    outdir: outDir, publicPath, entryNames: '[name]-[hash]', chunkNames: '[name]-[hash]',
    bundle: true, splitting: true, format: 'esm', minify: true,
    platform: 'browser', target: 'es2022', tsconfig,
    define: { __WEBUI_DEV__: 'false' }, metafile: true,
    plugins: [esbuildProjection({ manifest })],
  });
  const built = build({ ...options, projectionManifests: [manifest] });
  const routed = build({ ...options, entry: 'routed.html', projectionManifests: [manifest] });
  const independent = build({ ...options, entry: 'other-document.html' });
  if (built.lazyScriptsModule?.content !== first.lazyScriptsModule.content
    || routed.lazyScriptsModule?.content !== first.lazyScriptsModule.content) {
    throw new Error('Generated installer changed after projection.');
  }
  const protocol = new Protocol(built.protocol, { plugin: 'webui' });
  const routeProtocol = new Protocol(routed.protocol, { plugin: 'webui' });
  const outputs: Record<string, {
    bytes: number; gzip: number; brotli: number;
    inputs: string[]; imports: { path: string; kind: string }[];
  }> = {};
  let application = '';
  let routeApplication = '';
  for (const [file, details] of Object.entries(output.metafile.outputs)) {
    const url = publicPath + relative(outDir, resolve(file)).split(sep).join('/');
    const bytes = readFileSync(resolve(file));
    outputs[url] = {
      bytes: details.bytes, gzip: gzipSync(bytes).length, brotli: brotliCompressSync(bytes).length,
      inputs: Object.entries(details.inputs).filter(([, value]) => value.bytesInOutput > 0).map(([name]) => name.replaceAll('\\', '/')),
      imports: details.imports,
    };
    if (details.entryPoint?.endsWith('application.ts')) application = url;
    if (details.entryPoint?.endsWith('lazy-routes/index.ts')) routeApplication = url;
  }
  if (!application || !routeApplication) throw new Error('Lazy pages fixture has no application output.');
  const state = {
    pageA: false, pageB: false, help: '', sibling: false, nested: false, warm: false,
    message: 'Page-owned value', count: 0, title: 'Home from SSR',
  };
  const html = protocol.render({
    ...state, $webui: { bodyEnd: `<script type="module" src="${application}"></script>` },
  }, { entry: 'page.html' }).toString();
  const assets = new Map<string, [string, string]>();
  for (const pack of [...built.lazyPackFiles, ...routed.lazyPackFiles]) {
    assets.set(`/lazy-pages/${pack.path}`, [pack.content, 'application/json']);
  }
  for (let i = 0; i < built.cssFiles.length; i += 2) {
    assets.set(`/lazy-pages/${built.cssFiles[i]}`, [built.cssFiles[i + 1], 'text/css']);
  }
  const graph = JSON.stringify({
    application, routeApplication, outputs,
    registrationBytes: Buffer.byteLength(first.lazyScriptsModule.content),
    registration: first.lazyScriptsModule.content,
    independentEntryRegistration: independent.lazyScriptsModule?.content,
    packs: built.lazyPackFiles.map(pack => ({ region: pack.region, path: `/lazy-pages/${pack.path}`, scripts: pack.scripts, stylesheets: pack.stylesheets })),
    routePacks: routed.lazyPackFiles.map(pack => ({ region: pack.region, path: `/lazy-pages/${pack.path}`, scripts: pack.scripts, stylesheets: pack.stylesheets })),
  });
  writeFileSync(resolve(outDir, 'graph.json'), graph);
  return ({ req, url, send }): boolean => {
    if (['/lazy-routes/', '/lazy-routes/preview', '/lazy-routes/other'].includes(url.pathname)) {
      if (req.headers.accept === 'application/json') {
        const inventory = req.headers['x-webui-inventory'];
        send(200, routeProtocol.renderPartial(
          state, 'routed.html', url.pathname, typeof inventory === 'string' ? inventory : '',
        ), 'application/json');
      } else {
        send(200, routeProtocol.render({
          ...state, $webui: { bodyEnd: `<script type="module" src="${routeApplication}"></script>` },
        }, { entry: 'routed.html', requestPath: url.pathname }), 'text/html; charset=utf-8');
      }
      return true;
    }
    if (url.pathname === '/lazy-pages/fixture.html') {
      send(200, html, 'text/html; charset=utf-8');
      return true;
    }
    if (url.pathname === '/lazy-pages/graph') {
      send(200, graph, 'application/json');
      return true;
    }
    const asset = assets.get(url.pathname);
    if (!asset) return false;
    send(200, asset[0], asset[1]);
    return true;
  };
}
