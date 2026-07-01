// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Bundle-size harness for framework reachability checks.
 *
 * The probes intentionally model the important client graphs:
 * - root barrel: what a normal app import keeps alive
 * - auto element: the HTML-only runtime path
 * - authored probe: a component that uses decorators and events
 * - html-only probe: a static app that only imports the framework root
 *
 * This lets framework refactors prove they changed shipped bytes, not just
 * source layout. The script is dev-only and never participates in app bundles.
 */

import { brotliCompressSync, gzipSync } from 'node:zlib';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '..');

const sharedOptions = {
  bundle: true,
  format: 'esm',
  minify: true,
  platform: 'browser',
  target: 'es2022',
  treeShaking: true,
  write: false,
};

const probes = [
  {
    name: 'root barrel',
    entryPoints: [resolve(root, 'src/index.ts')],
  },
  {
    name: 'auto element internal',
    entryPoints: [resolve(root, 'src/auto-element.ts')],
  },
  {
    name: 'authored probe',
    stdin: {
      contents: `
        import { WebUIElement, observable, attr } from './src/index.ts';
        class SizeProbe extends WebUIElement {
          @attr label = '';
          @observable count = 0;
          onClick = () => { this.count++; };
        }
        SizeProbe.define('size-probe');
      `,
      loader: 'ts',
      resolveDir: root,
    },
  },
  {
    name: 'html-only probe',
    stdin: {
      contents: `import './src/index.ts';`,
      loader: 'ts',
      resolveDir: root,
    },
  },
];

function kb(bytes) {
  return `${(bytes / 1024).toFixed(2)} KB`;
}

const rows = [];
for (const probe of probes) {
  const { name, ...options } = probe;
  const result = await build({
    ...sharedOptions,
    ...options,
  });
  const code = result.outputFiles[0].contents;
  rows.push({
    bundle: name,
    minified: kb(code.length),
    gzip: kb(gzipSync(code).length),
    brotli: kb(brotliCompressSync(code).length),
  });
}

console.table(rows);
