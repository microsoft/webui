// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

interface VariantOption {
  value: string;
  activeClass: string;
  unavailable: boolean;
}

interface OptionGroup {
  name: string;
  values: VariantOption[];
}

export class MpVariantSelector extends WebUIElement {
  @observable optionGroups: OptionGroup[] = [];

  onClick(e: MouseEvent): void {
    const target = e.target;
    if (!(target instanceof Element)) {
      return;
    }

    const btn = target.closest('[data-action="select-variant"]') as HTMLElement | null;
    if (!btn || (btn as HTMLButtonElement).disabled) return;

    const group = btn.getAttribute('data-group') ?? '';
    const value = btn.getAttribute('data-value') || '';
    if (!group || !value) {
      return;
    }

    this.$emit('variant-select', { group, value });
  }
}

MpVariantSelector.define('mp-variant-selector');
