// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

export class CbStatCard extends WebUIElement {
  @attr icon = '';
  @attr value = '';
  @attr label = '';
}

CbStatCard.define('cb-stat-card');
