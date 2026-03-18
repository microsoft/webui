// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class MpAddToCart extends RenderableFASTElement(FASTElement) {
  @attr handle!: string;
  @attr({ attribute: 'product-title' }) productTitle!: string;
  @attr price!: string;
  @attr gradient!: string;
  @attr({ attribute: 'image-url' }) imageUrl!: string;
  @attr({ attribute: 'default-color' }) defaultColor = '';
  @attr({ attribute: 'default-size' }) defaultSize = '';
  @attr({ attribute: 'current-path' }) currentPath = '/';
  @observable selectedColor = '';
  @observable selectedSize = '';
  @observable canSubmit!: boolean;
  private clickHandler = (event: Event): void => { this.onClick(event as MouseEvent); };
  private variantHandler = (): void => { this.scheduleSelectionSync(); };

  connectedCallback(): void {
    super.connectedCallback();
    this.addEventListener('click', this.clickHandler);
    const root = this.getRootNode();
    if (root instanceof ShadowRoot) {
      root.addEventListener('variant-select', this.variantHandler as EventListener);
    }
    this.scheduleSelectionSync();
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    this.removeEventListener('click', this.clickHandler);
    const root = this.getRootNode();
    if (root instanceof ShadowRoot) {
      root.removeEventListener('variant-select', this.variantHandler as EventListener);
    }
  }

  setInitialState(state: Record<string, unknown>): void {
    if (typeof state.handle === 'string') this.handle = state.handle;
    if (state.productTitle !== undefined) this.productTitle = String(state.productTitle);
    if (typeof state.price === 'string') this.price = state.price;
    if (typeof state.gradient === 'string') this.gradient = state.gradient;
    if (typeof state.imageUrl === 'string') this.imageUrl = state.imageUrl;
    if (typeof state.defaultColor === 'string') this.defaultColor = state.defaultColor;
    if (typeof state.defaultSize === 'string') this.defaultSize = state.defaultSize;
    if (typeof state.currentPath === 'string') this.currentPath = state.currentPath;
    this.selectedColor = this.defaultColor;
    this.selectedSize = this.defaultSize;
    this.updateSelectionState();
  }

  async prepare(): Promise<void> {
    this.handle = this.getAttribute('handle') || '';
    this.productTitle = this.getAttribute('product-title') || '';
    this.price = this.getAttribute('price') || '';
    this.gradient = this.getAttribute('gradient') || '';
    this.imageUrl = this.getAttribute('image-url') || '';
    this.defaultColor = this.getAttribute('default-color') || '';
    this.defaultSize = this.getAttribute('default-size') || '';
    this.currentPath = this.getAttribute('current-path') || '/';
    this.selectedColor = this.defaultColor;
    this.selectedSize = this.defaultSize;
    this.updateSelectionState();
  }

  private onClick(event: MouseEvent): void {
    const button = this.findPathElement(event, '.add-to-cart-btn');
    if (!button) {
      return;
    }

    event.preventDefault();
    if (!this.canSubmit) {
      return;
    }
    void this.submitCart();
  }

  private updateSelectionState(): void {
    let color = '';
    let size = '';
    let hasMissingSelection = false;

    const pageRoot = this.getRootNode();
    const selector = pageRoot instanceof ShadowRoot
      ? pageRoot.querySelector('mp-variant-selector')
      : null;

    if (selector) {
      const groups = (selector as unknown as { optionGroups: { name: string; values: { value: string; activeClass: string }[] }[] }).optionGroups;
      if (groups?.length) {
        for (const g of groups) {
          const active = g.values.find(v => v.activeClass === 'active');
          if (active) {
            if (g.name.toUpperCase().includes('COLOR')) color = active.value;
            else if (g.name.toUpperCase().includes('SIZE')) size = active.value;
          } else {
            hasMissingSelection = true;
          }
        }
      } else {
        this.canSubmit = true;
      }
    } else {
      this.canSubmit = true;
    }

    this.selectedColor = color || this.defaultColor;
    this.selectedSize = size || this.defaultSize;
    this.canSubmit = !hasMissingSelection;
  }

  private scheduleSelectionSync(): void {
    void new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
      .then(() => this.updateSelectionState());
  }

  async onSubmit(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    await this.submitCart();
  }

  private async submitCart(): Promise<void> {
    this.updateSelectionState();
    if (!this.canSubmit) {
      return;
    }

    const response = await fetch('/cart/add', {
      method: 'POST',
      headers: {
        Accept: 'application/json',
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        handle: this.handle,
        color: this.selectedColor,
        size: this.selectedSize,
        redirectTo: this.currentPath,
        openCart: true,
      }),
      credentials: 'same-origin',
    });
    if (!response.ok) {
      return;
    }

    const state = await response.json() as Record<string, unknown>;
    document.dispatchEvent(new CustomEvent('commerce:cart-state', { detail: state }));
  }

  private findPathElement(event: Event, selector: string): Element | null {
    for (const target of event.composedPath()) {
      if (target instanceof Element && target.matches(selector)) {
        return target;
      }
    }

    return null;
  }
}

MpAddToCart.defineAsync({
  name: 'mp-add-to-cart',
  templateOptions: 'defer-and-hydrate',
});
