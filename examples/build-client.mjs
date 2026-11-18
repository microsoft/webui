// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Run the native browser builder for application-owned Node build scripts.
 */
import { createRequire } from "node:module";
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const workspaceRoot = realpathSync(fileURLToPath(new URL("../", import.meta.url)));

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
  const { bundleClient, prepareBuild } = await import(
    resolveImportEntry(require, "@microsoft/webui", "./build")
  );
  const { resolve } = await import(
    resolveImportEntry(require, "@microsoft/webui", "./platform.js")
  );
  buildTools = { bundleClient, prepareBuild, resolveBinary: () => resolve("bin") };
  return buildTools;
}

/** Compile templates once, bundle their generated inputs, then finalize projection. */
export async function runWebUIBuild({
  outDir = "dist",
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
  const client = await runWebUIClientBuild({
    outDir,
    ...clientOptions,
  });
  const result = prepared.finish({ projectionManifests: [client.manifestPath] });
  mkdirSync(outDir, { recursive: true });
  for (let i = 0; i < result.cssFiles.length; i += 2) {
    writeFileSync(path.join(outDir, result.cssFiles[i]), result.cssFiles[i + 1]);
  }
  for (const pack of result.lazyPackFiles) {
    const file = path.join(outDir, pack.path);
    mkdirSync(path.dirname(file), { recursive: true });
    if (!existsSync(file) || readFileSync(file, "utf8") !== pack.content) {
      writeFileSync(file, pack.content);
    }
  }
  if (metafilePath) writeFileSync(metafilePath, result.metafile);
  for (const warning of result.warnings) console.warn(warning);
  const protocol = path.join(outDir, "protocol.bin");
  const staged = `${protocol}.${process.pid}.tmp`;
  writeFileSync(staged, result.protocol);
  renameSync(staged, protocol);
  return { result, client };
}

export async function runWebUIClientBuild(options = {}) {
  const { bundleClient, resolveBinary } = await loadBuildTools();
  const watch = process.argv.includes("--watch");
  const buildOptions = {
    project: workspaceRoot,
    entries: { index: path.resolve("src/index.ts") },
    outDir: path.resolve("dist"),
    format: "esm",
    development: watch,
    minify: !watch,
    sourcemap: watch ? "file" : "none",
    ...options,
  };
  buildOptions.outDir = path.resolve(buildOptions.outDir);
  if (watch) {
    return watchNativeClient(buildOptions, resolveBinary());
  }
  const result = await bundleClient(buildOptions);
  for (const warning of result.warnings) console.warn(`[webui] ${warning.code}: ${warning.message}`);
  return result;
}

/** Only process/data hosting lives here; Rust owns watching and rebuilding. */
async function watchNativeClient(options, binary) {
  if (!binary || !path.isAbsolute(binary)) {
    throw new Error("A native WebUI binary is required; set WEBUI_BINARY_PATH to its absolute path.");
  }
  const { write, manifestPath, ...config } = options;
  if (write === false) throw new Error("Native watch publishes assets; write:false is not supported.");
  config.project = realpathSync(config.project);
  config.outDir = path.relative(config.project, config.outDir);
  const temporary = mkdtempSync(path.join(tmpdir(), "webui-client-watch-"));
  const configPath = path.join(temporary, "bundle.json");
  writeFileSync(configPath, JSON.stringify(config));
  const args = ["bundle", "--config", configPath, "--watch", "--control-stdin"];
  if (manifestPath) args.push("--manifest", path.resolve(config.project, manifestPath));
  const child = spawn(binary, args, { stdio: ["pipe", "pipe", "pipe"] });
  child.stdout.pipe(process.stdout, { end: false });
  child.stderr.pipe(process.stderr, { end: false });
  let stopping = false;
  let started = false;
  let disposed;
  const spawned = new Promise((resolve, reject) => {
    child.once("spawn", () => {
      started = true;
      resolve();
    });
    child.once("error", reject);
  });
  child.stdin.on("error", error => {
    if (!stopping) console.error(error);
  });
  const exited = new Promise(resolve => {
    child.once("close", (code, signal) => {
      process.removeListener("SIGINT", interrupt);
      process.removeListener("SIGTERM", terminate);
      rmSync(temporary, { recursive: true, force: true });
      if (!stopping && started) process.exitCode = code || 1;
      resolve({ code, signal });
    });
  });
  const dispose = () => {
    if (disposed) return disposed;
    stopping = true;
    child.stdin.end();
    disposed = (async () => {
      const terminate = setTimeout(() => child.kill("SIGTERM"), 5_000);
      const force = setTimeout(() => child.kill("SIGKILL"), 10_000);
      terminate.unref();
      force.unref();
      try {
        const status = await exited;
        if (status.code !== 0 || status.signal) {
          throw new Error(`Native client watcher did not stop cleanly (${status.signal ?? status.code}).`);
        }
      } finally {
        clearTimeout(terminate);
        clearTimeout(force);
      }
    })();
    return disposed;
  };
  const stop = code => {
    void dispose().catch(error => console.error(error)).finally(() => { process.exitCode = code; });
  };
  const interrupt = () => stop(130);
  const terminate = () => stop(143);
  process.once("SIGINT", interrupt);
  process.once("SIGTERM", terminate);
  try {
    await spawned;
  } catch (error) {
    await dispose().catch(() => {});
    throw error;
  }
  return { process: child, dispose };
}

const invokedPath = process.argv[1];
if (
  invokedPath &&
  import.meta.url === pathToFileURL(path.resolve(invokedPath)).href
) {
  await runWebUIClientBuild();
}
