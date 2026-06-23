// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

export class AssetBadge extends WebUIElement {
  @attr label = '';
}

AssetBadge.define('asset-badge');
