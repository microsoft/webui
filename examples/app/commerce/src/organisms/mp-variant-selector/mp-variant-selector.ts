// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

interface VariantOption {
  value: string;
  active: boolean;
  unavailable: boolean;
}

interface OptionGroup {
  name: string;
  values: VariantOption[];
}

export class MpVariantSelector extends WebUIElement {
  @observable optionGroups: OptionGroup[] = [];

  onVariantClick(e: MouseEvent): void {
    const btn = e.currentTarget;
    if (!(btn instanceof HTMLButtonElement) || btn.disabled) {
      return;
    }

    const group = btn.getAttribute('data-group') ?? '';
    const value = btn.getAttribute('data-value') ?? '';
    if (!group || !value) {
      return;
    }

    this.$emit('variant-select', { group, value });
  }
}

MpVariantSelector.define('mp-variant-selector');
