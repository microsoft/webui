// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class CbContactForm extends RenderableFASTElement(FASTElement) {
  @attr({ attribute: 'form-title' }) formTitle = 'Add Contact';
  @attr({ attribute: 'edit-id' }) editId = '';
  @attr({ attribute: 'first-name' }) firstName = '';
  @attr({ attribute: 'last-name' }) lastName = '';
  @attr email = '';
  @attr phone = '';
  @attr company = '';
  @attr address = '';
  @attr group = '';
  @attr notes = '';
  @observable selectedGroup = '';
  @observable groups?: string[];

  private listenersAttached!: boolean;

  setInitialState(state: Record<string, unknown>): void {
    this.formTitle = String(state.formTitle ?? 'Add Contact');
    if (Array.isArray(state.groups)) {
      this.groups = state.groups as string[];
    }
    if (state.id) {
      this.editId = String(state.id);
      this.firstName = String(state.firstName ?? '');
      this.lastName = String(state.lastName ?? '');
      this.email = String(state.email ?? '');
      this.phone = String(state.phone ?? '');
      this.company = String(state.company ?? '');
      this.address = String(state.address ?? '');
      this.group = String(state.group ?? '');
      this.notes = String(state.notes ?? '');
      this.selectedGroup = this.group.toLowerCase();
    } else if (this.groups && this.groups.length > 0) {
      this.selectedGroup = this.groups[0];
    }
    requestAnimationFrame(() => this.syncRadioSelection());
  }

  selectedGroupChanged(): void {
    this.syncRadioSelection();
  }

  connectedCallback(): void {
    super.connectedCallback();
    if (this.listenersAttached) return;
    this.listenersAttached = true;
    this.addEventListener('click', (e: Event) => {
      this.onClick(e as MouseEvent);
    });
  }

  async prepare(): Promise<void> {
    const sr = this.shadowRoot;
    if (!sr) return;

    // Recover groups from SSR'd radios for hydration
    const groups: string[] = [];
    for (const el of sr.querySelectorAll('input[type="radio"][name="group"]')) {
      const g = (el as HTMLInputElement).value;
      if (g) groups.push(g);
    }
    if (groups.length > 0) this.groups = groups;
    this.selectedGroup = this.group.toLowerCase() || (groups.length > 0 ? groups[0] : '');
    this.syncRadioSelection();

    // Textarea content needs imperative setting (browsers treat it as raw text)
    const ta = sr.querySelector('.notes-input') as HTMLTextAreaElement | null;
    if (ta && this.notes) ta.value = this.notes;
  }

  private syncRadioSelection(): void {
    for (const radio of this.shadowRoot?.querySelectorAll('input[type="radio"][name="group"]') || []) {
      (radio as HTMLInputElement).checked = (radio as HTMLInputElement).value === this.selectedGroup;
    }
  }

  onClick(e: MouseEvent): void {
    const target = e.composedPath()[0] as HTMLElement;
    const action = target.closest('[data-action]')?.getAttribute('data-action');
    if (action === 'cancel') {
      this.dispatchEvent(new CustomEvent('form-cancel', { bubbles: true, composed: true }));
    } else if (action === 'save') {
      const formData = this.collectFormData();
      if (formData) {
        this.dispatchEvent(new CustomEvent('form-save', {
          bubbles: true, composed: true, detail: formData,
        }));
      }
    }
  }

  private collectFormData(): Record<string, string> | null {
    const inputs = this.shadowRoot!.querySelectorAll('input.field-input');
    const data: Record<string, string> = {};
    for (const input of inputs) {
      const name = (input as HTMLInputElement).name || '';
      data[name] = (input as HTMLInputElement).value || '';
    }
    const checked = this.shadowRoot!.querySelector('input[type="radio"][name="group"]:checked') as HTMLInputElement | null;
    data.group = checked?.value || this.selectedGroup || '';
    const textarea = this.shadowRoot!.querySelector('.notes-input') as HTMLTextAreaElement;
    data.notes = textarea?.value || '';
    if (this.editId) data.id = this.editId;
    return data;
  }
}

CbContactForm.defineAsync({
  name: 'cb-contact-form',
  templateOptions: 'defer-and-hydrate',
});
