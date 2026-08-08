// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * @microsoft/webui-framework — lightweight Web Component runtime with SSR hydration.
 *
 * Provides a reactive base class, decorators, and hydration utilities for
 * building Web Components that work with WebUI's server-side rendering pipeline.
 *
 * @example
 * ```ts
 * import { WebUIElement, observable, attr } from '@microsoft/webui-framework';
 *
 * class MyCounter extends WebUIElement {
 *   @attr count = 0;
 *   @observable label = 'Count';
 * }
 * MyCounter.define('my-counter');
 * ```
 *
 * @packageDocumentation
 */

import { installTemplateElementRuntime } from './static-host.js';

// Set up compiler-owned dormant template hosts. The default entry deliberately
// carries no dependency on the streaming coordinator: streaming apps import the
// separate `@microsoft/webui-framework/streaming.js` entry (see
// `streaming-entry.ts`) to install it, so a non-streaming app never loads it.
setTimeout(installTemplateElementRuntime, 0);

export { WebUIElement } from './element.js';
export { observable, attr } from './decorators.js';
export { getTemplate, registerTemplateData } from './template.js';
export type { TemplateMeta } from './template.js';
export {
  installComponentStyles,
  prepareComponentStyles,
  registerComponentStyles,
} from './element/styles.js';
export type {
  ComponentStyleResource,
  ComponentStyles,
} from './element/styles.js';
export { hydrationStart, hydrationEnd } from './lifecycle.js';
