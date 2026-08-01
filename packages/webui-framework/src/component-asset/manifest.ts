// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/** State payload returned by a lazy component data loader. */
export type ComponentAssetState = Record<string, unknown>;

/** Manifest entry for one lazy component root. */
export interface ComponentAssetManifestEntry<Data extends ComponentAssetState = ComponentAssetState> {
  /** Static component asset module emitted by `webui build --emit-component-assets`. */
  asset: string | URL;
  /** Shared chunk URLs to modulepreload before importing the root asset. */
  modulepreload?: readonly (string | URL)[];
  /** JavaScript module that defines/registers the custom element class. */
  module?: () => Promise<unknown>;
  /** Optional data request kicked off in parallel with asset/module loading. */
  data?: () => Promise<Data>;
}

/** Map of component tag name to lazy asset metadata. */
export type ComponentAssetManifest = Record<string, ComponentAssetManifestEntry>;

/** In-flight or completed work for one lazy component root. */
export interface ComponentAssetPreload<Data extends ComponentAssetState = ComponentAssetState> {
  /** Static WebUI template/style asset registration. */
  asset: Promise<void>;
  /** Optional JavaScript module import. */
  module?: Promise<unknown>;
  /** Optional data request. */
  data?: Promise<Data>;
}

/** Options for creating a lazy component element from a manifest entry. */
export interface ComponentAssetCreateOptions {
  /** Wait for data before returning the element. Defaults to false. */
  awaitData?: boolean;
  /** Maximum time to wait for data when awaitData is true. */
  dataTimeoutMs?: number;
}

/** Loader returned by `defineComponentAssets`. */
export interface ComponentAssetRegistry {
  /** Start asset, module, and optional data work for a component. */
  preload<Data extends ComponentAssetState = ComponentAssetState>(tag: string): ComponentAssetPreload<Data>;
  /** Create a component element and apply loaded data via setState(), if present. */
  create(tag: string, options?: ComponentAssetCreateOptions): Promise<HTMLElement>;
}
