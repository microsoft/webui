// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Public streaming-hydration entry point.
 *
 * Importing this module installs the progressive streaming hydration
 * coordinator as a one-time side effect. It is published separately (as
 * `@microsoft/webui-framework/streaming.js`, mirroring
 * `@microsoft/webui-framework/component-asset.js`) so the default framework
 * entry (`@microsoft/webui-framework`) carries no static or dynamic dependency
 * on the coordinator — a non-streaming app never loads `streaming.js` at all.
 *
 * A streaming app imports this entry once, before its component side-effect
 * imports, so the coordinator is installed synchronously and the streaming
 * gate opens before any authored `.define()` call runs in the same module
 * graph:
 *
 * ```ts
 * import '@microsoft/webui-framework/streaming.js';
 * import './my-component.js';
 * ```
 *
 * Installing is cheap and idempotent: on a non-streaming page it costs exactly
 * one cached meta-tag query and returns.
 */

import { installStreamingCoordinator } from './streaming.js';

installStreamingCoordinator();
