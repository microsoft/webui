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

export * from './core.js';
export * from './styles-entry.js';
