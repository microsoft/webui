// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

interface ContactFormData {
  firstName: string;
  lastName: string;
  email: string;
  phone: string;
  company: string;
  address: string;
  notes: string;
  group: string;
  id?: string;
}

export class CbContactForm extends WebUIElement {
  @observable formTitle = 'Add Contact';
  @observable editId = '';
  @observable firstName = '';
  @observable lastName = '';
  @observable email = '';
  @observable phone = '';
  @observable company = '';
  @observable address = '';
  @observable notes = '';
  @observable selectedGroup = '';
  @observable groups: string[] = [];

  groupsChanged(): void {
    if (!this.selectedGroup && this.groups.length > 0) {
      this.selectedGroup = this.groups[0] ?? '';
    }
  }

  onFieldInput(event: Event): void {
    const input = event.currentTarget;
    if (!(input instanceof HTMLInputElement)) {
      return;
    }

    switch (input.name) {
      case 'firstName':
        this.firstName = input.value;
        break;
      case 'lastName':
        this.lastName = input.value;
        break;
      case 'email':
        this.email = input.value;
        break;
      case 'phone':
        this.phone = input.value;
        break;
      case 'company':
        this.company = input.value;
        break;
      case 'address':
        this.address = input.value;
        break;
      default:
        break;
    }
  }

  onNotesInput(event: Event): void {
    const textarea = event.currentTarget;
    if (textarea instanceof HTMLTextAreaElement) {
      this.notes = textarea.value;
    }
  }

  onGroupChange(event: Event): void {
    const input = event.currentTarget;
    if (input instanceof HTMLInputElement && input.type === 'radio') {
      this.selectedGroup = input.value;
    }
  }

  onCancel(): void {
    this.$emit('form-cancel');
  }

  onSave(): void {
    this.$emit('form-save', this.collectFormData());
  }

  private collectFormData(): ContactFormData {
    const data: ContactFormData = {
      firstName: this.firstName,
      lastName: this.lastName,
      email: this.email,
      phone: this.phone,
      company: this.company,
      address: this.address,
      notes: this.notes,
      group: this.selectedGroup || this.groups[0] || '',
    };

    if (this.editId) {
      data.id = this.editId;
    }

    return data;
  }
}

CbContactForm.define('cb-contact-form');
