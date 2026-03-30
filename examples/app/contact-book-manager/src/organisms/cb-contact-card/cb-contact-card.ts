// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

export class CbContactCard extends WebUIElement {
  @attr id = '';
  @attr firstName = '';
  @attr lastName = '';
  @attr email = '';
  @attr phone = '';
  @attr company = '';
  @attr group = '';
  @attr favorite = 'false';
  @attr initials = '';
  @attr avatarColor = '';
  @attr notes = '';
  @attr address = '';

  onClick(): void {
    this.$emit('select-contact', { id: this.id });
  }
}

CbContactCard.define('cb-contact-card');
