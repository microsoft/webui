// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Run one application-owned esbuild invocation with WebUI projection enabled.
 */
export async function runWebUIClientBuild(
  esbuild,
  esbuildProjection,
  options,
) {
  const watch = process.argv.includes("--watch");
  const color = process.argv.includes("--color=true");
  const { projectionManifest, ...esbuildOptions } = options;
  const buildOptions = {
    ...esbuildOptions,
    color,
    metafile: true,
    plugins: [
      ...(esbuildOptions.plugins ?? []),
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
