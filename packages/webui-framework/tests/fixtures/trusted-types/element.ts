// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';

export class TestTrustedTypes extends WebUIElement {
  @observable rawHtml!: string;
  @observable count!: number;
  @observable show!: boolean;
  @observable items!: string[];
}

TestTrustedTypes.define('test-trusted-types');
