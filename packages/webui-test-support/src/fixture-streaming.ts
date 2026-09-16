// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { build as buildClient } from 'esbuild';
import { build as buildProtocol, Protocol } from '@microsoft/webui';
import type { StreamStep } from '@microsoft/webui';
import { esbuildProjection } from '@microsoft/webui/projection.js';
import { relative, resolve, sep } from 'node:path';
import type { FixtureRequestContext } from './fixture-server.js';

interface StreamingFixtureOptions {
  fixturePath: string;
  outDir: string;
  tsconfig: string;
}

interface CoordinatorAsset {
  src: string;
  imports: string[];
}

/** Public assets plus internal output identities used by browser regressions. */
export interface StreamingFixtureAssets {
  coordinator: CoordinatorAsset;
  application: string;
  templateHostRuntime: string;
}

function attribute(value: string): string {
  return value.replaceAll('&', '&amp;').replaceAll('"', '&quot;').replaceAll('<', '&lt;');
}

/** Build a real split application and serve independently paced native streams. */
export async function prepareStreamingFixture({
  fixturePath,
  outDir,
  tsconfig,
}: StreamingFixtureOptions): Promise<(context: FixtureRequestContext) => boolean> {
  const publicPath = '/dist/streaming-bootstrap/';
  const applicationPath = resolve(fixturePath, 'application.ts');
  const streamingPath = resolve(fixturePath, 'streaming.ts');
  const projectionPath = resolve(outDir, 'webui-projection.json');
  const built = await buildClient({
    absWorkingDir: fixturePath,
    entryPoints: { application: applicationPath, streaming: streamingPath },
    outdir: outDir,
    publicPath,
    entryNames: '[name]-[hash]',
    chunkNames: 'chunks/[name]-[hash]',
    bundle: true,
    splitting: true,
    format: 'esm',
    platform: 'browser',
    target: 'es2022',
    minify: true,
    tsconfig,
    define: { __WEBUI_DEV__: 'false' },
    metafile: true,
    plugins: [
      esbuildProjection({ manifest: projectionPath }),
    ],
  });
  if (!built.metafile) throw new Error('Streaming fixture has no esbuild output metadata.');
  const servedUrl = (output: string): string =>
    publicPath + relative(outDir, resolve(fixturePath, output)).split(sep).join('/');
  let application: string | undefined;
  let coordinatorOutput: string | undefined;
  let templateHostRuntime: string | undefined;
  for (const [output, details] of Object.entries(built.metafile.outputs)) {
    if (!details.entryPoint) continue;
    const href = servedUrl(output);
    if (resolve(fixturePath, details.entryPoint) === applicationPath) application = href;
    if (resolve(fixturePath, details.entryPoint) === streamingPath) coordinatorOutput = output;
    if (
      details.entryPoint.endsWith('/static-host.js') ||
      details.entryPoint.endsWith('/static-host.ts')
    ) {
      if (templateHostRuntime) throw new Error('Streaming fixture bundled two template-host runtimes.');
      templateHostRuntime = href;
    }
  }
  if (!application || !templateHostRuntime || !coordinatorOutput) {
    throw new Error('Streaming fixture is missing an application, streaming, or lazy template-host entry.');
  }
  const closure = new Set([coordinatorOutput]);
  for (const output of closure) {
    for (const dependency of built.metafile.outputs[output].imports) {
      if (dependency.kind === 'import-statement' && !dependency.external) closure.add(dependency.path);
    }
  }
  closure.delete(coordinatorOutput);
  const imports = [...closure].sort((a, b) =>
    built.metafile!.outputs[b].bytes - built.metafile!.outputs[a].bytes ||
    Buffer.compare(Buffer.from(a), Buffer.from(b)),
  );
  const assets: StreamingFixtureAssets = {
    coordinator: { src: servedUrl(coordinatorOutput), imports: imports.map(servedUrl) },
    application,
    templateHostRuntime,
  };
  const assetJson = JSON.stringify(assets);
  let head = '';
  for (const href of assets.coordinator.imports) {
    head += `<link rel="modulepreload" href="${attribute(href)}">`;
  }
  head += `<script type="module" async src="${attribute(assets.coordinator.src)}"></script>`;
  const applicationScript =
    `<script type="module" src="${attribute(application)}" fetchpriority="low"></script>`;
  const earlyApplicationScript =
    `<script type="module" async src="${attribute(application)}"></script>`;
  const protocols = new Map<string, Protocol>();
  for (const entry of ['page.html', 'static.html']) {
    const compiled = buildProtocol({
      appDir: resolve(fixturePath, 'src'),
      entry,
      plugin: 'webui',
      css: 'style',
      projectionManifests: [projectionPath],
    });
    if (!compiled.protocol?.length) {
      throw new Error(`Streaming fixture compiled no protocol for ${entry}.`);
    }
    protocols.set(entry, new Protocol(compiled.protocol, { plugin: 'webui' }));
  }
  const state = { count: 0, label: 'Ready' };
  const pending = new Map<string, () => void>();
  const staticProtocol = protocols.get('static.html');
  if (!staticProtocol) throw new Error('Streaming fixture has no static component protocol.');
  const staticTemplates = staticProtocol.renderComponentTemplates(['test-stream-static'], '');

  return ({ req, res, url, send }): boolean => {
    if (url.pathname === '/streaming-bootstrap/assets') {
      send(200, assetJson, 'application/json');
      return true;
    }
    if (url.pathname === '/streaming-bootstrap/static-templates') {
      send(200, staticTemplates, 'application/json');
      return true;
    }
    if (url.pathname === '/streaming-bootstrap/release' && req.method === 'POST') {
      pending.get(url.searchParams.get('id') ?? '')?.();
      send(204, '');
      return true;
    }
    if (url.pathname !== '/streaming-bootstrap/fixture.html') return false;
    const staticOnly = url.searchParams.has('static');
    const entry = staticOnly ? 'static.html' : 'page.html';
    const protocol = protocols.get(entry);
    if (!protocol) throw new Error(`Streaming fixture has no protocol for ${entry}.`);
    const early = url.searchParams.has('early');
    const options = {
      entry,
      nonce: 'streaming-fixture',
      headInject: early ? head + earlyApplicationScript : head,
      bodyInject: staticOnly || early ? '' : applicationScript,
    };
    if (url.searchParams.has('ordinary')) {
      send(200, protocol.render({
        ...state,
        $webui: { headEnd: options.headInject, bodyEnd: options.bodyInject },
      }, { entry }), 'text/html; charset=utf-8');
      return true;
    }
    res.setHeader(
      'Content-Security-Policy',
      "script-src 'self' 'nonce-streaming-fixture'; style-src 'self' 'unsafe-inline'; object-src 'none'" +
        (url.searchParams.has('trusted') ? "; require-trusted-types-for 'script'; trusted-types webui" : ''),
    );
    const id = url.searchParams.get('id') ?? '';
    if (!id || id.length > 128 || pending.has(id) || pending.size >= 32) {
      send(400, 'A unique streaming fixture id is required (at most 32 concurrent streams).');
      return true;
    }
    res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
    const session = protocol.streamResponse(options);
    let step: StreamStep = session.start(state);
    res.write(step.bytes);
    const commit = (): void => {
      const boundary = step.boundary;
      if (!boundary) throw new Error('Streaming fixture expected a pending boundary.');
      const updatable = boundary.name === 'first';
      step = session.resume(boundary.instanceId, state, updatable ? 'updatable' : 'final');
      res.write(step.bytes);
      if (updatable) res.write(session.update(boundary.instanceId, { count: 1 }));
    };
    if (!url.searchParams.has('before') && step.boundary) commit();
    const cleanup = (): void => {
      clearTimeout(timeout);
      pending.delete(id);
      res.off('close', cleanup);
    };
    const finish = (): void => {
      cleanup();
      while (!step.done) {
        if (step.boundary) {
          commit();
        } else {
          step = session.advance();
          res.write(step.bytes);
        }
      }
      res.end();
    };
    const timeout = setTimeout(() => res.destroy(), 30_000);
    timeout.unref();
    pending.set(id, finish);
    res.once('close', cleanup);
    return true;
  };
}
