import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class MpPrice extends RenderableFASTElement(FASTElement) {
  @attr value = '';
  @attr size = 'md';
  @attr variant = 'pill';
  @attr({ attribute: 'currency-code' }) currencyCode = 'USD';

  async prepare(): Promise<void> {
    this.value = this.getAttribute('value') || '';
    this.size = this.getAttribute('size') || 'md';
    this.variant = this.getAttribute('variant') || 'pill';
    this.currencyCode = this.getAttribute('currency-code') || 'USD';
  }
}

MpPrice.defineAsync({
  name: 'mp-price',
  templateOptions: 'defer-and-hydrate',
});
