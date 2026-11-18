// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { addonInstallHelp, loadBinding } from "./binding.js";

/** Options passed directly to the shared native Rolldown browser compiler. */
export interface ClientBundleOptions {
  /** Containing boundary for physical browser sources and emitted files. */
  project: string;
  /** Output entry names mapped to their authored source paths. */
  entries: Record<string, string>;
  /** Output directory, relative to project or absolute inside it. */
  outDir: string;
  /** Optional version-matched framework source package. */
  framework?: string;
  development?: boolean;
  minify?: boolean;
  sourcemap?: "none" | "file" | "inline" | "hidden";
  format?: "esm" | "iife";
  target?: string;
  defines?: Record<string, string>;
  aliases?: Record<string, string>;
  external?: string[];
  entryNames?: string;
  chunkNames?: string;
  publicPath?: string;
  loaders?: Record<string, "js" | "ts" | "jsx" | "tsx" | "json" | "text" | "css" | "file" | "dataurl">;
  virtualModules?: Record<string, string>;
  tsconfig?: string;
  /** Start each dynamic target and its native static-closure chunks together. */
  parallelDynamicImports?: boolean;
  /** Publish final files and the native proof; false returns owned buffers only. */
  write?: boolean;
  /** Proof destination, defaulting to outDir/webui-projection.json. */
  manifestPath?: string;
}

/** One native bundler import edge. */
export interface ClientOutputImport {
  path: string;
  kind: "import-statement" | "dynamic-import";
  external: boolean;
}

/** Input/output evidence from the same native bundler invocation. */
export interface ClientMetafile {
  inputs: Record<string, {
    bytes: number;
    source: string;
    opaque: boolean;
    syntax?: string;
    imports: ClientOutputImport[];
  }>;
  outputs: Record<string, {
    bytes: number;
    entryPoint?: string;
    imports: ClientOutputImport[];
    inputs: Record<string, { renderedBytesBeforeMinify: number }>;
    parallelDynamicImports?: Record<string, string[]>;
  }>;
}

/** Exact source/output proof produced by the Rust projection compiler. */
export interface ClientProjectionManifest {
  schema: string;
  producer: { name: string; version: string };
  adapter: { name: string; bundler: string };
  root: string;
  analysisHash: string;
  buildId: string;
  inputs: Record<string, string>;
  outputs: Record<string, string>;
  components: Record<string, {
    module: string;
    outputs: string[];
    hydrationKeys: string[];
    navigationKeys: string[];
  }>;
  entryClosures?: Record<string, string[]>;
}

/** Final browser files and their authoritative native build evidence. */
export interface ClientBundle {
  files: Array<{ path: string; contents: Buffer }>;
  manifestPath: string;
  manifest: ClientProjectionManifest;
  metafile: ClientMetafile;
  warnings: ClientBuildWarning[];
}

/** A nonfatal compiler advisory, never a suppressed projection failure. */
export interface ClientBuildWarning {
  code: string;
  message: string;
}

interface NativeClientBinding {
  bundleClient(options: string, manifestPath?: string, write?: boolean): Promise<{
    files: ClientBundle["files"];
    manifestPath: string;
    manifestJson: string;
    metafileJson: string;
    warnings: ClientBuildWarning[];
  }>;
}

/**
 * Compile browser assets, projection and final-byte proof entirely in Rust.
 * Callers own exclusive publication to a destination; use write:false for
 * independently staged or concurrently compiled outputs.
 */
export async function bundleClient(options: ClientBundleOptions): Promise<ClientBundle> {
  const { manifestPath, write, ...build } = options;
  const native = loadBinding<Partial<NativeClientBinding>>("build-addon");
  if (typeof native.bundleClient !== "function") {
    throw new Error(
      `[webui] The loaded build addon has no native browser compiler. ${addonInstallHelp("build-addon")}`,
    );
  }
  const result = await native.bundleClient(JSON.stringify(build), manifestPath, write);
  return {
    files: result.files,
    manifestPath: result.manifestPath,
    manifest: JSON.parse(result.manifestJson) as ClientProjectionManifest,
    metafile: JSON.parse(result.metafileJson) as ClientMetafile,
    warnings: result.warnings,
  };
}
