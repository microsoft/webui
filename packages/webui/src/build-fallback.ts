// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import { tmpdir } from "node:os";
import nodePath from "node:path";
import { readComponentAssetFiles } from "./component-assets.js";
import type {
  BuildOptions,
  BuildResult,
  BuildStats,
} from "./index.js";

/** Build through the CLI when the platform-native Node addon is unavailable. */
export function buildWithCli(
  binPath: string,
  options: BuildOptions,
): BuildResult {
  const componentAssetRoots = options.componentAssetRoots ?? [];
  if (options.componentAssetMetafile && componentAssetRoots.length === 0) {
    throw new Error(
      "[webui] componentAssetMetafile requires at least one componentAssetRoot.",
    );
  }
  if (!options.outDir) {
    throw new Error(
      "[webui] CLI fallback requires outDir so build outputs can be read back.",
    );
  }

  const outDir = options.outDir;
  const args = buildArguments(options);
  let temporaryMetafileDirectory: string | undefined;
  let graphMetafilePath: string | undefined;
  if (componentAssetRoots.length > 0) {
    args.push("--emit-component-assets", componentAssetRoots.join(","));
    temporaryMetafileDirectory = fs.mkdtempSync(
      nodePath.join(tmpdir(), "webui-component-assets-"),
    );
    graphMetafilePath = nodePath.join(
      temporaryMetafileDirectory,
      "component-assets.meta.json",
    );
    args.push("--metafile", graphMetafilePath);
  }
  if (options.cssFileNameTemplate !== undefined) {
    args.push("--asset-file-name-template", options.cssFileNameTemplate);
  }
  if (options.cssPublicBase) {
    args.push("--css-public-base", options.cssPublicBase);
  }
  if (options.theme) args.push("--theme", options.theme);
  args.push("--out", outDir);

  try {
    execFileSync(binPath, args, { stdio: "inherit" });
    const protocol = fs.readFileSync(nodePath.join(outDir, "protocol.bin"));
    const graphMetafile = graphMetafilePath
      ? fs.readFileSync(graphMetafilePath, "utf8")
      : undefined;
    return {
      protocol,
      cssFiles: [],
      componentAssetFiles: readComponentAssetFiles(outDir, graphMetafile),
      componentAssetMetafile: options.componentAssetMetafile
        ? graphMetafile
        : undefined,
      warnings: [],
      stats: emptyStats(),
    };
  } finally {
    if (temporaryMetafileDirectory) {
      fs.rmSync(temporaryMetafileDirectory, { recursive: true, force: true });
    }
  }
}

function buildArguments(options: BuildOptions): string[] {
  const args = ["build", options.appDir ?? "."];
  if (options.entry) args.push("--entry", options.entry);
  if (options.css) args.push("--css", options.css);
  if (options.plugin) args.push("--plugin", options.plugin);
  if (options.components) {
    for (const component of options.components) {
      args.push("--components", component);
    }
  }
  if (options.projectionManifests) {
    for (const manifest of options.projectionManifests) {
      args.push("--projection-manifest", manifest);
    }
  }
  if (
    options.projectionManifestObjects &&
    options.projectionManifestObjects.length > 0
  ) {
    throw new Error(
      "[webui] Inline projection manifest objects require the native addon; write the manifest and pass projectionManifests when using the CLI fallback.",
    );
  }
  return args;
}

function emptyStats(): BuildStats {
  return {
    durationMs: 0,
    fragmentCount: 0,
    componentCount: 0,
    cssFileCount: 0,
    protocolSizeBytes: 0,
    tokenCount: 0,
  };
}
