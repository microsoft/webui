// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { installTemplateRegistry } from './template-registry.js';
import { installStylesRuntime } from './style-installation.js';

installTemplateRegistry();
installStylesRuntime();

export {
  installComponentStyles,
  prepareComponentStyles,
  registerComponentStyles,
} from './element/styles.js';
export type {
  ComponentStyleResource,
  ComponentStyles,
} from './element/style-catalog.js';
