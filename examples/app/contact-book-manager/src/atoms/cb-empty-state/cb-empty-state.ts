// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

export class CbEmptyState extends WebUIElement {
  @attr icon = '📭';
  @attr title = '';
  @attr message = '';
}

CbEmptyState.define('cb-empty-state');
