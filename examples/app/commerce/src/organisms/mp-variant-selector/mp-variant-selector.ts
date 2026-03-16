// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

interface VariantOption {
  value: string;
  grp: string;
  activeClass: string;
  unavailable: boolean;
}

interface OptionGroup {
  name: string;
  values: VariantOption[];
}

export class MpVariantSelector extends RenderableFASTElement(FASTElement) {
  @observable optionGroups!: OptionGroup[];

  private clickHandler = (e: Event): void => { this.onClick(e as MouseEvent); };

  connectedCallback(): void {
    super.connectedCallback();
    // Use host element for event delegation — connectedCallback fires before
    // FAST hydration replaces shadow DOM content, so listeners on shadowRoot
    // get detached. Host-level listeners survive hydration because native click
    // events compose through shadow DOM (composed: true).
    this.addEventListener('click', this.clickHandler);
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    this.removeEventListener('click', this.clickHandler);
  }

  async prepare(): Promise<void> {
    if (Array.isArray(this.optionGroups) && this.optionGroups.length > 0) {
      return;
    }

    const groups: OptionGroup[] = [];
    const groupEls = this.shadowRoot!.querySelectorAll('.option-group');

    groupEls.forEach(groupEl => {
      const name = groupEl.querySelector('.option-name')?.textContent?.trim() || '';
      const values: VariantOption[] = [];

      groupEl.querySelectorAll('.pill').forEach(pill => {
        values.push({
          value: pill.getAttribute('data-value') || pill.textContent?.trim() || '',
          grp: name,
          activeClass: pill.classList.contains('active') ? 'active' : '',
          unavailable: (pill as HTMLButtonElement).disabled,
        });
      });

      groups.push({ name, values });
    });

    this.optionGroups = groups;
  }

  onClick(e: MouseEvent): void {
    const target = e.composedPath()[0] as HTMLElement;
    const btn = target.closest('[data-action="select-variant"]') as HTMLElement | null;
    if (!btn || (btn as HTMLButtonElement).disabled) return;

    // Read group name from parent DOM structure (avoids FAST template binding issues)
    const groupEl = btn.closest('dd')?.parentElement;
    const group = groupEl?.querySelector('dt')?.textContent?.trim() || '';
    const value = btn.getAttribute('data-value') || '';

    // Build plain objects — spreading FAST observable objects copies internal
    // _-prefixed backing fields that override surface property values.
    this.optionGroups = this.optionGroups.map(g => ({
      name: g.name,
      values: g.name !== group
        ? g.values.map(v => ({ value: v.value, grp: v.grp, activeClass: v.activeClass, unavailable: v.unavailable }))
        : g.values.map(v => ({
            value: v.value,
            grp: v.grp,
            activeClass: v.value === value ? 'active' : '',
            unavailable: v.unavailable,
          })),
    }));

    this.$emit('variant-select', { group, value });
  }
}

MpVariantSelector.defineAsync({
  name: 'mp-variant-selector',
  templateOptions: 'defer-and-hydrate',
});
