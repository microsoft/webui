// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';

const JAVASCRIPT_OUTPUT = /\.(?:mjs|cjs|js)$/;

function outputFile(filePath, text) {
  const contents = new TextEncoder().encode(text);
  return {
    path: filePath,
    contents,
    hash: createHash('sha256').update(contents).digest('base64'),
    get text() { return text; },
  };
}

/** Optimize final JavaScript before projection binds the emitted output bytes. */
export function minifyClient() {
  return {
    name: 'webui-production-minification',
    setup(build) {
      const options = build.initialOptions;
      const write = options.write !== false;
      // Include postprocessing in esbuild's cache identity; Terser drops this comment.
      const identity = createHash('sha256')
        .update(readFileSync(new URL(import.meta.url)))
        .update(readFileSync(new URL('../pnpm-lock.yaml', import.meta.url)))
        .update(JSON.stringify([
          options.format, options.keepNames, options.minifyIdentifiers,
          options.minifySyntax, options.minifyWhitespace, options.sourcemap,
          options.sourcesContent,
        ]))
        .digest('hex');
      options.banner = {
        ...options.banner,
        js: `// WebUI production optimizer ${identity}\n${options.banner?.js ?? ''}`,
      };
      options.write = false;
      options.minify = false;
      const workingDirectory = options.absWorkingDir ?? process.cwd();
      build.onEnd(async result => {
        if (result.errors.length) return;
        const { minify } = await import('terser');
        const files = result.outputFiles;
        if (!files || !result.metafile) {
          throw new Error('Production minification requires esbuild output files and a metafile.');
        }
        const indices = new Map(files.map((file, index) => [file.path, index]));
        for (let index = 0; index < files.length; index++) {
          const file = files[index];
          if (!JAVASCRIPT_OUTPUT.test(file.path)) continue;
          const mapIndex = indices.get(`${file.path}.map`);
          const sourceMap = options.sourcemap;
          const inline = sourceMap === 'inline' || sourceMap === 'both';
          const optimized = await minify(
            { [path.basename(file.path)]: file.text },
            {
              module: options.format === 'esm',
              compress: options.minifySyntax === false ? false : { passes: 3 },
              mangle: options.minifyIdentifiers !== false,
              keep_classnames: options.keepNames,
              keep_fnames: options.keepNames,
              format: {
                // One diagnostic character must not double retained script-source storage.
                ascii_only: true,
                beautify: options.minifyWhitespace === false,
                comments: 'some',
              },
              sourceMap: sourceMap ? {
                content: mapIndex === undefined ? 'inline' : files[mapIndex].text,
                filename: path.basename(file.path),
                url: inline ? 'inline'
                  : sourceMap === 'external' ? undefined : `${path.basename(file.path)}.map`,
                includeSources: options.sourcesContent !== false,
              } : false,
            },
          );
          if (optimized.code === undefined) {
            throw new Error(`Production minification did not return JavaScript for ${file.path}.`);
          }
          files[index] = outputFile(file.path, optimized.code);
          if (mapIndex !== undefined) {
            if (typeof optimized.map !== 'string') {
              throw new Error(`Production minification did not return a source map for ${file.path}.`);
            }
            files[mapIndex] = outputFile(files[mapIndex].path, optimized.map);
          }
        }
        for (const [filePath, output] of Object.entries(result.metafile.outputs)) {
          const index = indices.get(path.resolve(workingDirectory, filePath));
          if (index === undefined) continue;
          if (JAVASCRIPT_OUTPUT.test(filePath)) {
            // Terser cannot attribute its rewritten bytes to esbuild inputs.
            output.webui = {
              inputBytesInOutput: 'before-minification',
              bytesBeforeMinification: output.bytes,
            };
          }
          output.bytes = files[index].contents.byteLength;
        }
        if (write) {
          for (const file of files) {
            await mkdir(path.dirname(file.path), { recursive: true });
            await writeFile(file.path, file.contents);
          }
        }
      });
    },
  };
}
