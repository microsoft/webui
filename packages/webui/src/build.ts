// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { addonInstallHelp, loadBinding } from "./binding.js";

export { bundleClient } from "./bundle.js";
export type {
  ClientBundleOptions,
  ClientBundle,
  ClientMetafile,
  ClientOutputImport,
  ClientProjectionManifest,
  ClientBuildWarning,
} from "./bundle.js";

/** Options for compiling a WebUI application. */
export interface BuildOptions {
  /** Application folder containing templates. */
  appDir: string;
  /** Entry HTML file name (default: "index.html"). */
  entry?: string;
  /** CSS delivery strategy (default: "link"). */
  css?: "link" | "style" | "module";
  /** Default DOM strategy for unwrapped components (default: "shadow"). */
  dom?: "shadow" | "light";
  /** Merge shared CSS trees; not supported with css: "module". */
  cssBundle?: boolean;
  /** Parser plugin name. */
  plugin?: string;
  /** Additional component sources, as npm packages or local paths. */
  components?: string[];
  /** Root component tags emitted as static .webui.js ESM assets. */
  componentAssetRoots?: string[];
  /** Return component/lazy-pack cost metadata and measure Brotli sizes. */
  metafile?: boolean;
  /** Link-mode CSS filename template using [name], [hash] and [ext]. */
  cssFileNameTemplate?: string;
  /**
   * Public URL or path ending in "/" for emitted stylesheets and packs.
   * Module preload hints retain their original paths. A request may override
   * this default with the reserved { $webui: { assetBase } } state member.
   */
  assetBase?: string;
  /**
   * Custom-element module specifiers overriding discovered sibling scripts.
   * Generated lazy.webui.js imports these specifiers verbatim.
   */
  componentModules?: Record<string, string>;
  /**
   * Directory for generated modules; defaults to appDir. Relative generated
   * imports start here. Write modules before bundling the browser entry.
   */
  modulesOut?: string;
  /** Design token theme: a JSON path or npm package name. */
  theme?: string;
  /** Completed projection manifest paths, merged in order. */
  projectionManifests?: string[];
  /** Inline manifests with logical paths anchoring root/stale validation. */
  projectionManifestObjects?: Array<{ path: string; manifest: unknown }>;
}

/** Native compilation statistics. */
export interface BuildStats {
  /** Build duration in milliseconds. */
  durationMs: number;
  /** Total protocol fragments. */
  fragmentCount: number;
  /** Registered components. */
  componentCount: number;
  /** Emitted CSS files. */
  cssFileCount: number;
  /** Serialized protocol length. */
  protocolSizeBytes: number;
  /** Unique CSS tokens. */
  tokenCount: number;
}

/** One lazy-region template pack and its cost report. */
export interface LazyPackFile {
  /** Content-addressed file name inside _webui/packs/. */
  fileName: string;
  /** JSON pack content. */
  content: string;
  /** Asset-base-relative _webui/packs/<fileName> path. */
  path: string;
  /** Region identity, <owner>::lazy:@<n>. */
  region: string;
  /** Owning component. */
  owner: string;
  /** Prefetch trigger. */
  trigger: "intent" | "idle" | "eager" | "demand";
  /** Content hash from the filename. */
  hash: string;
  /** Content length in bytes. */
  bytes: number;
  /** Brotli quality-11 size, when metafile was requested. */
  brotliBytes?: number;
  /** Modules of scripted components introduced by this region. */
  scripts: string[];
  /** Asset-base-relative Link stylesheets; empty for Style and Module CSS. */
  stylesheets: string[];
}

/** Declarations accompanying a generated browser module. */
export interface GeneratedDeclaration {
  /** File name, such as lazy.webui.d.ts. */
  fileName: string;
  /** TypeScript declaration source. */
  content: string;
}

/** Generated ES module for the application bundler. */
export interface GeneratedModule {
  /** File name in modulesOut. */
  fileName: string;
  /** JavaScript source. */
  content: string;
  /** TypeScript declarations written beside the module. */
  declaration: GeneratedDeclaration;
}

/** Compiler output to publish with the browser bundle. */
export interface BuildResult {
  /** Serialized protobuf protocol. */
  protocol: Buffer;
  /** CSS files as alternating filename/content pairs. */
  cssFiles: string[];
  /** Static component modules as alternating filename/content pairs. */
  componentAssetFiles: string[];
  /** Publish each pack under its path. */
  lazyPackFiles: LazyPackFile[];
  /** Write this module and its declaration into modulesOut before bundling. */
  lazyScriptsModule?: GeneratedModule;
  /** Component asset and lazy-pack metafile JSON, when requested. */
  metafile?: string;
  /** Nonfatal plain-text compiler advisories. */
  warnings: string[];
  /** Native build statistics. */
  stats: BuildStats;
}

/** Completed native proofs used to finalize retained templates. */
export type FinishBuildOptions = Pick<
  BuildOptions,
  "projectionManifests" | "projectionManifestObjects"
>;

/** Single-use compilation retained while its generated modules are bundled. */
export interface PreparedBuild {
  /** Projection-independent generated behavior imports. */
  readonly lazyScriptsModule?: GeneratedModule;
  /** Generated component modules, as filename/content pairs. */
  readonly componentAssetFiles: string[];
  /** Projection-independent packs and their region metadata. */
  readonly lazyPackFiles: LazyPackFile[];
  /** Atomically publish compiler-generated inputs into modulesOut. */
  writeModules(): void;
  /** Consume this snapshot, validate completed proofs and serialize it. */
  finish(options?: FinishBuildOptions): BuildResult;
}

type NativeBuildOptions = Omit<BuildOptions, "projectionManifestObjects"> & {
  projectionManifestObjects?: Array<{ path: string; json: string }>;
};

interface NativePreparedBuild {
  readonly lazyScriptsModule: GeneratedModule | null;
  readonly componentAssetFiles: string[];
  readonly lazyPackFiles: LazyPackFile[];
  writeModules(): void;
  finish(options: Pick<
    NativeBuildOptions,
    "projectionManifests" | "projectionManifestObjects"
  >): BuildResult;
}

interface NativeBuildAddon {
  build(options: NativeBuildOptions): BuildResult;
  prepareBuild(options: NativeBuildOptions): NativePreparedBuild;
}

function nativeBuildOptions<T extends FinishBuildOptions>(options: T) {
  const { projectionManifestObjects, ...nativeOptions } = options;
  return {
    ...nativeOptions,
    projectionManifestObjects: projectionManifestObjects?.map(({ path, manifest }) => ({
      path,
      json: JSON.stringify(manifest),
    })),
  };
}

/** Compile templates, styles and protocol without loading the handler addon. */
export function build(options: BuildOptions): BuildResult {
  const native = loadBinding<NativeBuildAddon>("build-addon");
  if (typeof native.build !== "function") {
    throw new Error(
      `[webui] Native build addon is incompatible: build() is required. ${addonInstallHelp("build-addon")}`,
    );
  }
  return native.build(nativeBuildOptions(options));
}

/** Compile once, publish generated inputs, then finalize after native bundling. */
export function prepareBuild(options: BuildOptions): PreparedBuild {
  const native = loadBinding<NativeBuildAddon>("build-addon");
  if (typeof native.prepareBuild !== "function") {
    throw new Error(
      `[webui] Native build addon is incompatible: prepareBuild() is required. ${addonInstallHelp("build-addon")}`,
    );
  }
  const prepared = native.prepareBuild(nativeBuildOptions(options));
  return {
    get lazyScriptsModule() { return prepared.lazyScriptsModule ?? undefined; },
    get componentAssetFiles() { return prepared.componentAssetFiles; },
    get lazyPackFiles() { return prepared.lazyPackFiles; },
    writeModules() { prepared.writeModules(); },
    finish(options = {}) {
      return prepared.finish(nativeBuildOptions(options));
    },
  };
}
