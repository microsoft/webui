// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { mkdir, rename, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { build } from 'esbuild';
import { minify } from 'terser';
import { prepareBuild } from '@microsoft/webui';
import { esbuildProjection } from '@microsoft/webui/projection.js';

// Projection must bind the final, ASCII-encoded production bytes, not an
// intermediate bundle. onEnd hooks run in plugin registration order.
function productionOutput() {
  return {
    name: 'production-output',
    setup(builder) {
      builder.onEnd(async result => {
        if (result.errors.length) return;
        for (const file of result.outputFiles ?? []) {
          if (!file.path.endsWith('.js')) continue;
          const optimized = await minify(file.text, {
            module: true,
            compress: { passes: 3 },
            format: { ascii_only: true, comments: 'some' },
          });
          if (optimized.code === undefined) throw new Error(`Could not minify ${file.path}`);
          file.contents = new TextEncoder().encode(optimized.code);
          const relative = path.relative(process.cwd(), file.path).split(path.sep).join('/');
          result.metafile.outputs[relative].bytes = file.contents.byteLength;
        }
      });
    },
  };
}

export async function buildFrontend(development = false) {
  const stage = '.webui/build';
  const backup = '.webui/previous';
  await rm(stage, { recursive: true, force: true });
  await mkdir(`${stage}/public`, { recursive: true });
  const prepared = prepareBuild({
    appDir: 'src',
    entry: 'index.html',
    plugin: 'webui',
    css: 'link',
    assetBase: '/assets/',
    modulesOut: '.webui/modules',
  });
  prepared.writeModules();
  const manifest = `${stage}/projection.json`;
  const client = await build({
    entryPoints: ['src/index.ts'],
    outdir: `${stage}/public`,
    bundle: true,
    format: 'esm',
    target: 'es2022',
    charset: 'ascii',
    // Hoist legal notices before Terser can remove the nodes they annotate.
    legalComments: 'eof',
    tsconfig: 'tsconfig.json',
    write: false,
    metafile: true,
    define: { __WEBUI_DEV__: development ? 'true' : 'false' },
    plugins: [
      ...(!development ? [productionOutput()] : []),
      esbuildProjection({ manifest }),
    ],
  });
  for (const file of client.outputFiles) {
    await mkdir(path.dirname(file.path), { recursive: true });
    await writeFile(file.path, file.contents);
  }
  const result = prepared.finish({ projectionManifests: [manifest] });
  for (let index = 0; index < result.cssFiles.length; index += 2) {
    const file = path.join(stage, 'public', result.cssFiles[index]);
    await mkdir(path.dirname(file), { recursive: true });
    await writeFile(file, result.cssFiles[index + 1]);
  }
  await writeFile(`${stage}/protocol.bin`, result.protocol);
  for (const warning of result.warnings) console.warn(warning);
  await rm(backup, { recursive: true, force: true });
  let previous = false;
  try {
    await rename('dist', backup);
    previous = true;
  } catch (error) {
    if (error.code !== 'ENOENT') throw error;
  }
  try {
    await rename(stage, 'dist');
  } catch (error) {
    if (previous) await rename(backup, 'dist');
    throw error;
  }
  await rm(backup, { recursive: true, force: true });
}
