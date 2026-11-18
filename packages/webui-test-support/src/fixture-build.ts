// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { existsSync, readdirSync, realpathSync, rmSync } from 'node:fs';
import { extname, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

import { bundleClient, type ClientBundle, type ClientBundleOptions } from '@microsoft/webui/build';

/** Physical root shared by fixture sources, workspace packages, and outputs. */
export const workspaceRoot = realpathSync(fileURLToPath(new URL('../../../', import.meta.url)));

/** Keep native advisories visible in every browser-fixture build. */
export async function bundleFixtureClient(options: ClientBundleOptions): Promise<ClientBundle> {
  const built = await bundleClient(options);
  for (const warning of built.warnings) console.warn(`[webui] ${warning.code}: ${warning.message}`);
  return built;
}

export interface FixtureBuildOptions {
  fixturesRoot: string;
  entryFileName: string;
  outDir: string;
  tsconfig: string;
  emptyMessage?: string;
  extraBuilds?: ClientBundleOptions[];
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

  const entries = Object.fromEntries(entryPoints.map(entry => [
    relative(fixturesRoot, entry).slice(0, -extname(entry).length).split(sep).join('/'),
    entry,
  ]));
  const built = await bundleFixtureClient({
    project: workspaceRoot,
    entries,
    format: 'esm',
    outDir,
    target: 'es2022',
    tsconfig,
    development: true,
    minify: false,
  });

  for (const extraBuild of extraBuilds) {
    const extraManifest = resolve(extraBuild.project,
      extraBuild.manifestPath ?? resolve(extraBuild.project, extraBuild.outDir, 'webui-projection.json'));
    if (extraManifest === built.manifestPath) {
      throw new Error('Fixture extra builds must set a distinct manifestPath to preserve the authored entry proof.');
    }
    await bundleFixtureClient(extraBuild);
  }
  return built.manifestPath;
}
