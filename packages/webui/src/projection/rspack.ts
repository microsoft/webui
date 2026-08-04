// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Official Rspack adapter for WebUI state projection.
 *
 * This subpath is isolated from `@microsoft/webui/projection.js` so that
 * legacy/esbuild consumers never load Rspack code or acquire the optional
 * `@rspack/core` peer. The implementation uses type-only imports from
 * `@rspack/core` and reads runtime classes/constants/sources/version from the
 * compiler, so importing this subpath without the peer runtime stays isolated.
 */

export type {
  RspackProjectionOptions,
  RspackProjectionResult,
  RspackProjectionPlugin,
} from "./adapters/rspack.js";
export { rspackProjection } from "./adapters/rspack.js";
