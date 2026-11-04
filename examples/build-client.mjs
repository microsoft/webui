// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Run one application-owned esbuild invocation with WebUI projection enabled.
 */
import { createRequire } from "node:module";
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

let buildTools;

function resolveImportEntry(require, packageName, subpath) {
  const packageSegments = packageName.split("/");
  for (const searchPath of require.resolve.paths(packageName) ?? []) {
    const packageRoot = path.join(searchPath, ...packageSegments);
    const packageJsonPath = path.join(packageRoot, "package.json");
    if (!existsSync(packageJsonPath)) continue;
    const packageJson = JSON.parse(readFileSync(packageJsonPath, "utf8"));
    const target = packageJson.exports?.[subpath]?.import?.default;
    if (typeof target === "string") {
      return pathToFileURL(path.resolve(packageRoot, target)).href;
    }
  }
  throw new Error(
    `Cannot resolve ${packageName}${subpath.slice(1)} from ${process.cwd()}`,
  );
}

async function loadBuildTools() {
  if (buildTools) return buildTools;
  const require = createRequire(
    pathToFileURL(path.join(process.cwd(), "package.json")),
  );
  const esbuild = require(require.resolve("esbuild"));
  const projectionEntry = resolveImportEntry(
    require,
    "@microsoft/webui",
    "./projection.js",
  );
  const { esbuildProjection } = await import(projectionEntry);
  const { prepareBuild } = await import(resolveImportEntry(require, "@microsoft/webui", "."));
  buildTools = { esbuild, esbuildProjection, prepareBuild };
  return buildTools;
}

/** Compile templates once, bundle their generated inputs, then finalize projection. */
export async function runWebUIBuild({
  outdir = "dist",
  metafilePath,
  clientOptions = {},
  ...options
} = {}) {
  if (process.argv.includes("--watch")) {
    throw new Error("runWebUIBuild is a production build; use the existing client/server watchers for development.");
  }
  const { prepareBuild } = await loadBuildTools();
  const prepared = prepareBuild({
    appDir: "src",
    modulesOut: ".webui",
    ...options,
    metafile: Boolean(metafilePath),
  });
  prepared.writeModules();
  const manifest = path.join(outdir, "webui-projection.json");
  const client = await runWebUIClientBuild({
    outdir,
    ...clientOptions,
    projectionManifest: manifest,
  });
  const result = prepared.finish({ projectionManifests: [manifest] });
  mkdirSync(outdir, { recursive: true });
  for (let i = 0; i < result.cssFiles.length; i += 2) {
    writeFileSync(path.join(outdir, result.cssFiles[i]), result.cssFiles[i + 1]);
  }
  for (const pack of result.lazyPackFiles) {
    const file = path.join(outdir, pack.path);
    mkdirSync(path.dirname(file), { recursive: true });
    if (!existsSync(file) || readFileSync(file, "utf8") !== pack.content) {
      writeFileSync(file, pack.content);
    }
  }
  if (metafilePath) writeFileSync(metafilePath, result.metafile);
  for (const warning of result.warnings) console.warn(warning);
  const protocol = path.join(outdir, "protocol.bin");
  const staged = `${protocol}.${process.pid}.tmp`;
  writeFileSync(staged, result.protocol);
  renameSync(staged, protocol);
  return { result, client };
}

export async function runWebUIClientBuild(options = {}) {
  const { esbuild, esbuildProjection } = await loadBuildTools();
  const watch = process.argv.includes("--watch");
  const color = process.argv.includes("--color=true");
  const { projectionManifest, plugins = [], ...esbuildOptions } = options;
  const buildOptions = {
    entryPoints: ["src/index.ts"],
    outdir: "dist",
    bundle: true,
    format: "esm",
    splitting: true,
    minify: !watch,
    sourcemap: watch,
    ...esbuildOptions,
    color,
    metafile: true,
    plugins: [
      ...plugins,
      esbuildProjection(
        projectionManifest
          ? { manifest: projectionManifest }
          : undefined,
      ),
    ],
  };

  if (watch) {
    const context = await esbuild.context(buildOptions);
    await context.watch();
    return context;
  }
  return esbuild.build(buildOptions);
}

const invokedPath = process.argv[1];
if (
  invokedPath &&
  import.meta.url === pathToFileURL(path.resolve(invokedPath)).href
) {
  await runWebUIClientBuild();
}
