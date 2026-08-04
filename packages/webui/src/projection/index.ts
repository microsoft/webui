// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * `@microsoft/webui/projection.js` — build-only state projection compiler subpath.
 *
 * This subpath is intentionally separate from the root `@microsoft/webui`
 * entry so that render/build consumers do not load compiler or adapter code.
 *
 * Peer dependencies required by this subpath:
 *   - `typescript` ^6.0.3  (for TypeScript AST analysis)
 *   - `esbuild` ^0.28.1    (for the esbuild adapter only; other adapters have
 *                            their own optional peer requirements)
 *
 * Both peers are optional in `package.json` so users of the root build/render
 * API do not receive peer-missing warnings. Importing this subpath without the
 * required peer produces `PROJ-P001` or `PROJ-P002`.
 *
 * See DESIGN.md §"Bundler-Neutral State Projection Compiler" for the
 * authoritative specification.
 */

export * from "./core.js";
export * from "./testing.js";
export * from "./esbuild.js";
