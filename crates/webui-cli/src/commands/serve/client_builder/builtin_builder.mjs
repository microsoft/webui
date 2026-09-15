// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

async function createBuiltinBuilder({ appDir, outDir, clientEntry }) {
  const { createRequire } = await import("node:module");
  const { join, dirname } = await import("node:path");
  const { stat } = await import("node:fs/promises");
  const { pathToFileURL } = await import("node:url");
  const require = createRequire(pathToFileURL(join(appDir, "package.json")));
  let resolved;
  try {
    resolved = require.resolve("esbuild");
  } catch (error) {
    if (error?.code !== "MODULE_NOT_FOUND") throw error;
    throw new Error(
      "Cannot resolve esbuild from the application. Add esbuild to this project's devDependencies, then restart webui dev.",
      { cause: error },
    );
  }
  const esbuild = await import(pathToFileURL(resolved).href);
  if (typeof esbuild.context !== "function") {
    throw new Error(
      "This project's esbuild does not support persistent contexts. Use the esbuild version supported by @microsoft/webui.",
    );
  }
  let project = appDir;
  for (;;) {
    try {
      if ((await stat(join(project, "package.json"))).isFile()) break;
    } catch (error) {
      if (error?.code !== "ENOENT") throw error;
    }
    const parent = dirname(project);
    if (parent === project) {
      project = appDir;
      break;
    }
    project = parent;
  }
  const context = await esbuild.context({
    absWorkingDir: appDir,
    entryPoints: [clientEntry],
    outdir: outDir,
    bundle: true,
    format: "esm",
    sourcemap: true,
    target: "es2022",
    define: { __WEBUI_DEV__: "true" },
    logLevel: "warning",
  });
  return {
    watchPaths: [project],
    async rebuild() {
      await context.rebuild();
    },
    async dispose() {
      await context.dispose();
    },
  };
}
