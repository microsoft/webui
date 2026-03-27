// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

import '#atoms/mp-price/mp-price.js';
import '#organisms/mp-product-gallery/mp-product-gallery.js';
import '#organisms/mp-variant-selector/mp-variant-selector.js';
import '#organisms/mp-add-to-cart/mp-add-to-cart.js';
import '#organisms/mp-product-card/mp-product-card.js';

interface VariantOption {
  value: string;
  activeClass: string;
  unavailable: boolean;
}

interface OptionGroup {
  name: string;
  values: VariantOption[];
}

interface VariantSelectDetail {
  group?: string;
  value?: string;
}

function sameOptionGroups(left: OptionGroup[], right: OptionGroup[]): boolean {
  if (left.length !== right.length) {
    return false;
  }

  for (let index = 0; index < left.length; index += 1) {
    const leftGroup = left[index];
    const rightGroup = right[index];
    if (leftGroup.name !== rightGroup.name || leftGroup.values.length !== rightGroup.values.length) {
      return false;
    }

    for (let valueIndex = 0; valueIndex < leftGroup.values.length; valueIndex += 1) {
      const leftValue = leftGroup.values[valueIndex];
      const rightValue = rightGroup.values[valueIndex];
      if (leftValue.value !== rightValue.value
        || leftValue.activeClass !== rightValue.activeClass
        || leftValue.unavailable !== rightValue.unavailable) {
        return false;
      }
    }
  }

  return true;
}

export class MpPageProduct extends WebUIElement {
  @observable handle!: string;
  @observable productTitle!: string;
  @observable price!: string;
  @observable gradient!: string;
  @observable gradientAlt!: string;
  @observable imageUrl!: string;
  @observable imageAltUrl!: string;
  @observable compareAt!: string;
  @observable hasCompareAt!: boolean;
  @observable descriptionHtml!: string;
  @observable defaultColor!: string;
  @observable defaultSize!: string;
  @observable selectedColor!: string;
  @observable selectedSize!: string;
  @observable currentPath!: string;

  @observable images!: any[];
  @observable optionGroups!: OptionGroup[];
  @observable relatedProducts!: any[];

  onVariantSelect(event: Event): void {
    const detail = (event as CustomEvent<VariantSelectDetail>).detail;
    const group = detail.group?.trim().toLowerCase() ?? '';
    const value = detail.value ?? '';
    if (!group || !value) {
      return;
    }

    if (group.includes('color')) {
      this.selectedColor = value;
      return;
    }

    if (group.includes('size')) {
      this.selectedSize = value;
    }
  }

  optionGroupsChanged(): void {
    this.syncOptionGroups();
  }

  selectedColorChanged(): void {
    this.syncOptionGroups();
  }

  selectedSizeChanged(): void {
    this.syncOptionGroups();
  }

  defaultColorChanged(): void {
    this.syncOptionGroups();
  }

  defaultSizeChanged(): void {
    this.syncOptionGroups();
  }

  private syncOptionGroups(): void {
    const optionGroups = Array.isArray(this.optionGroups) ? this.optionGroups : [];
    const nextSelectedColor = this.selectedColor || this.defaultColor;
    const nextSelectedSize = this.selectedSize || this.defaultSize;

    if (this.selectedColor !== nextSelectedColor) {
      this.selectedColor = nextSelectedColor;
    }
    if (this.selectedSize !== nextSelectedSize) {
      this.selectedSize = nextSelectedSize;
    }

    const nextGroups = optionGroups.map((group) => {
      const activeValue = this.selectionForGroup(group.name, nextSelectedColor, nextSelectedSize);
      return {
        name: group.name,
        values: group.values.map((value) => ({
          value: value.value,
          unavailable: value.unavailable,
          activeClass: activeValue && value.value === activeValue ? 'active' : '',
        })),
      };
    });

    if (!sameOptionGroups(optionGroups, nextGroups)) {
      this.optionGroups = nextGroups;
    }
  }

  private selectionForGroup(name: string, selectedColor: string, selectedSize: string): string {
    const normalized = name.trim().toLowerCase();
    if (normalized.includes('color')) {
      return selectedColor;
    }
    if (normalized.includes('size')) {
      return selectedSize;
    }
    return '';
  }
}

MpPageProduct.define('mp-page-product');
