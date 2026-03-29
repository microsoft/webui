// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

export class CbBadge extends WebUIElement {
  @attr label = '';
  @attr variant = 'default';
}

CbBadge.define('cb-badge');
