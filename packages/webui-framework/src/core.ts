// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { installTemplateElementRuntime } from './static-host.js';

setTimeout(installTemplateElementRuntime, 0);

export { WebUIElement } from './element.js';
export { observable, attr } from './decorators.js';
export { getTemplate, registerTemplateData } from './template-registry.js';
export type { TemplateMeta } from './template-types.js';
export { hydrationStart, hydrationEnd } from './lifecycle.js';
