// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

export class CbContactDetail extends WebUIElement {
  @observable id!: string;
  @observable firstName!: string;
  @observable lastName!: string;
  @observable email!: string;
  @observable phone!: string;
  @observable company!: string;
  @observable group!: string;
  @observable favorite!: boolean;
  @observable initials!: string;
  @observable avatarColor!: string;
  @observable notes!: string;
  @observable address!: string;

  onEdit(): void {
    this.$emit('edit-contact', { id: this.id });
  }

  onToggleFavorite(): void {
    this.favorite = !this.favorite;
    this.$emit('toggle-favorite', { id: this.id });
  }

  onDelete(): void {
    this.$emit('delete-contact', { id: this.id });
  }

  onGoBack(): void {
    this.$emit('go-back');
  }
}

CbContactDetail.define('cb-contact-detail');
