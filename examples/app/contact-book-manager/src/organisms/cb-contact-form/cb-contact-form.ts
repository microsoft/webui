import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

const formGroupsStore = new WeakMap<object, string[]>();

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

  private listenersAttached!: boolean;

  connectedCallback(): void {
    super.connectedCallback();
    if (this.listenersAttached) return;
    this.listenersAttached = true;
    this.addEventListener('click', (e: Event) => {
      this.onClick(e as MouseEvent);
    });
  }

  async prepare(): Promise<void> {
    const groups: string[] = [];
    for (const el of this.shadowRoot!.querySelectorAll('.group-option')) {
      const g = el.getAttribute('data-group') || el.textContent || '';
      if (g) groups.push(g);
    }
    formGroupsStore.set(this, groups);
    this.selectedGroup = this.group || (groups.length > 0 ? groups[0] : '');
  }

  onClick(e: MouseEvent): void {
    const target = e.composedPath()[0] as HTMLElement;
    const action = target.closest('[data-action]')?.getAttribute('data-action');
    const groupBtn = target.closest('[data-group]');

    if (groupBtn) {
      this.selectedGroup = groupBtn.getAttribute('data-group') || '';
      // Update visual active state
      for (const btn of this.shadowRoot!.querySelectorAll('.group-option')) {
        btn.classList.toggle('active', btn.getAttribute('data-group') === this.selectedGroup);
      }
      return;
    }

    if (action === 'cancel') {
      this.dispatchEvent(new CustomEvent('form-cancel', { bubbles: true, composed: true }));
    } else if (action === 'save') {
      const formData = this.collectFormData();
      if (formData) {
        this.dispatchEvent(new CustomEvent('form-save', { bubbles: true, composed: true, detail: formData }));
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
    data.group = this.selectedGroup;
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
