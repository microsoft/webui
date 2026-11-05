// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '../../../src/runtime.js';
import '../../../src/styles-entry.js';
import { WebUIElement, observable } from '../../../src/index.js';

startRuntime();

export class TestTrustedTypes extends WebUIElement {
  @observable rawHtml!: string;
  @observable count!: number;
  @observable show!: boolean;
  @observable items!: string[];
}

TestTrustedTypes.define('test-trusted-types');
