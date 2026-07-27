// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { existsSync, readdirSync, rmSync } from 'node:fs';
import { resolve } from 'node:path';

import { build, type BuildOptions } from 'esbuild';
import { esbuildProjection } from '@microsoft/webui/projection.js';

export interface FixtureBuildOptions {
  fixturesRoot: string;
  entryFileName: string;
  outDir: string;
  tsconfig: string;
  emptyMessage?: string;
  extraBuilds?: BuildOptions[];
}

export function collectFixtureEntryPoints(
  fixturesRoot: string,
  entryFileName: string,
): string[] {
  return readdirSync(fixturesRoot, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => resolve(fixturesRoot, entry.name, entryFileName))
    .filter((entry) => existsSync(entry));
}

export async function buildFixtureEntries({
  fixturesRoot,
  entryFileName,
  outDir,
  tsconfig,
  emptyMessage,
  extraBuilds = [],
}: FixtureBuildOptions): Promise<string> {
  const entryPoints = collectFixtureEntryPoints(fixturesRoot, entryFileName);
  if (entryPoints.length === 0) {
    throw new Error(emptyMessage ?? `No fixture entry points found in ${fixturesRoot}`);
  }

  rmSync(outDir, { recursive: true, force: true });

  const projectionManifest = resolve(outDir, 'webui-projection.json');
  await build({
    entryPoints,
    bundle: true,
    format: 'iife',
    outdir: outDir,
    outbase: fixturesRoot,
    platform: 'browser',
    target: 'es2022',
    supported: { 'import-attributes': true },
    tsconfig,
    logLevel: 'info',
    metafile: true,
    plugins: [esbuildProjection({ manifest: projectionManifest })],
  });

  for (const extraBuild of extraBuilds) {
    await build({
      logLevel: 'info',
      ...extraBuild,
    });
  }
  return projectionManifest;
}
