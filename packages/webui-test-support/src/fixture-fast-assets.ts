// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { build, Protocol } from '@microsoft/webui';
import { build as bundle } from 'esbuild';
import { mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import type { FixtureRequestContext } from './fixture-server.js';

/** Exercise compiler-owned FAST links through SSR and the real resource consumer. */
export async function prepareFastAssets(
  root: string,
  outDir: string,
): Promise<(context: FixtureRequestContext) => boolean> {
  mkdirSync(outDir, { recursive: true });
  await bundle({
    entryPoints: [resolve(root, 'examples/app/todo-fast/tests/asset-base-entry.ts')],
    outfile: resolve(outDir, 'application.js'),
    bundle: true, format: 'esm', platform: 'browser', target: 'es2022',
    nodePaths: [resolve(root, 'packages/webui-delivery/node_modules')],
    tsconfig: resolve(root, 'tsconfig.json'),
  });
  const protocols = new Map<string, Protocol>();
  const css = new Map<string, string>();
  for (const version of ['2', '3']) {
    const compiled = build({
      appDir: resolve(root, 'packages/webui-framework/tests/fixtures/fast-assets/src'),
      plugin: `fast-v${version}`, css: 'link', assetBase: '/static/', entry: 'page.html',
    });
    protocols.set(version, new Protocol(compiled.protocol, { plugin: `fast-v${version}` }));
    for (let i = 0; i < compiled.cssFiles.length; i += 2) css.set(compiled.cssFiles[i], compiled.cssFiles[i + 1]);
  }
  return ({ url, send }): boolean => {
    const resource = url.pathname.startsWith('/fast-assets/templates/') || url.pathname.startsWith('/fast-assets/partial/');
    const version = resource ? url.pathname.split('/').at(-1)! : url.searchParams.get('version') ?? '3';
    const empty = !resource && url.searchParams.has('empty');
    const protocol = protocols.get(version);
    if (!protocol) return false;
    if (url.pathname.startsWith('/fast-assets/templates/')) {
      send(200, protocol.renderComponentTemplates(['review-fast-card'], ''), 'application/json');
      return true;
    }
    if (url.pathname.startsWith('/fast-assets/partial/')) {
      send(200, protocol.renderPartial({ count: 1 }, 'page.html', '/nested/', ''), 'application/json');
      return true;
    }
    if (url.pathname === '/fast-assets/nested/') {
      const base = url.searchParams.has('override') ? '/override/' : '/static/';
      send(200, protocol.render({
        count: 1, show: !empty,
        $webui: { assetBase: base, bodyEnd: '<script type="module" src="/dist/fast-assets/application.js"></script>' },
      }, { entry: 'page.html', requestPath: '/nested/' }), 'text/html; charset=utf-8');
      return true;
    }
    for (const base of ['/static/', '/override/']) {
      if (url.pathname.startsWith(base)) {
        const content = css.get(url.pathname.slice(base.length));
        if (content !== undefined) { send(200, content, 'text/css'); return true; }
      }
    }
    if (url.pathname === '/fast-assets/nested/authored.css') { send(200, '/* Authored URL stays document-relative. */', 'text/css'); return true; }
    return false;
  };
}
