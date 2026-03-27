// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

export class MpProductImage extends WebUIElement {
  @attr gradient = '';
  @attr({ attribute: 'image-url' }) imageUrl = '';
  @attr alt = '';
  @attr interactive = '';
  @attr loading = 'lazy';
  @attr decoding = 'async';
  @attr({ attribute: 'fetch-priority' }) fetchPriority = 'auto';
}

MpProductImage.define('mp-product-image');
