// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable, attr } from '@microsoft/webui-framework';

interface VariantOption {
  value: string;
  unavailable: boolean;
}

interface OptionGroup {
  name: string;
  selected: string;
  values: VariantOption[];
}

export class MpVariantSelector extends WebUIElement {
  @observable optionGroups: OptionGroup[] = [];

  onVariantClick(e: MouseEvent): void {
    const btn = e.currentTarget;
    if (!(btn instanceof HTMLButtonElement) || btn.disabled) return;

    const groupName = btn.getAttribute('data-group') ?? '';
    const value = btn.getAttribute('data-value') ?? '';
    if (!groupName || !value) return;

    // Update the group's selected value — the template condition
    // `opt.value == group.selected` reactively updates the ?active attr.
    for (const g of this.optionGroups) {
      if (g.name === groupName) {
        g.selected = value;
      }
    }
    this.optionGroups = [...this.optionGroups];

    this.$emit('variant-select', { group: groupName, value });
  }
}

MpVariantSelector.define('mp-variant-selector');
