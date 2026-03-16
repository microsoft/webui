// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class CbContactCard extends RenderableFASTElement(FASTElement) {
  @attr id = '';
  @attr({ attribute: 'first-name' }) firstName = '';
  @attr({ attribute: 'last-name' }) lastName = '';
  @attr email = '';
  @attr phone = '';
  @attr company = '';
  @attr group = '';
  @attr favorite = 'false';
  @attr initials = '';
  @attr({ attribute: 'avatar-color' }) avatarColor = '';
  @attr notes = '';
  @attr address = '';

  private listenersAttached!: boolean;

  connectedCallback(): void {
    super.connectedCallback();
    if (this.listenersAttached) return;
    this.listenersAttached = true;
    this.addEventListener('click', () => {
      this.onClick();
    });
  }

  onClick(): void {
    this.dispatchEvent(new CustomEvent('select-contact', { bubbles: true, composed: true, detail: { id: this.id } }));
  }
}

CbContactCard.defineAsync({
  name: 'cb-contact-card',
  templateOptions: 'defer-and-hydrate',
});
