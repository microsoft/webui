// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

export class CbAvatar extends WebUIElement {
  @attr initials = '';
  @attr color = '#6B7280';
  @attr size = 'md';
}

CbAvatar.define('cb-avatar');
