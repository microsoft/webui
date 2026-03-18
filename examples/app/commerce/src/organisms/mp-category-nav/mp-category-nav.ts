// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class MpCategoryNav extends RenderableFASTElement(FASTElement) {
  @attr({ attribute: 'all-active-class' }) allActiveClass = '';
  @attr({ attribute: 'current-label' }) currentCategoryLabel = 'All';
  @observable categories?: any[];
  private clickHandler = (e: Event): void => { this.onClick(e as MouseEvent); };
  private routeHandler = (): void => { this.closeMobileDropdown(); };

  connectedCallback(): void {
    super.connectedCallback();
    this.addEventListener('click', this.clickHandler);
    window.addEventListener('webui:route:navigated', this.routeHandler);
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    this.removeEventListener('click', this.clickHandler);
    window.removeEventListener('webui:route:navigated', this.routeHandler);
  }

  async prepare(): Promise<void> {
    this.allActiveClass = this.getAttribute('all-active-class') || '';
    this.currentCategoryLabel = this.getAttribute('current-label') || this.currentCategoryLabel || 'All';

    if (Array.isArray(this.categories)) {
      this.synccurrentCategoryLabel();
      return;
    }

    const sr = this.shadowRoot;
    if (!sr) return;
    const links = sr.querySelectorAll('.desktop-list .link');
    if (links.length <= 1) return;
    const cats: any[] = [];
    links.forEach((link) => {
      const element = link as HTMLElement;
      const handle = element.getAttribute('data-handle') || '';
      if (!handle) return;
      cats.push({
        handle,
        title: element.textContent?.trim() || '',
        count: 0,
        activeClass: element.classList.contains('active') ? 'active' : '',
      });
    });
    this.categories = cats;
    this.synccurrentCategoryLabel();
  }

  setInitialState(state: Record<string, unknown>): void {
    if (Array.isArray(state.categories)) {
      this.categories = state.categories as any[];
    }
    if (typeof state.allActiveClass === 'string') {
      this.allActiveClass = state.allActiveClass;
    }
    if (typeof state.currentCategoryLabel === 'string') {
      this.currentCategoryLabel = state.currentCategoryLabel;
    }
    this.synccurrentCategoryLabel();
    const view = this.$fastController?.view;
    if (view) {
      view.unbind();
      view.bind(this, view.context);
    }
  }

  categoriesChanged(): void {
    this.synccurrentCategoryLabel();
  }

  allActiveClassChanged(): void {
    this.synccurrentCategoryLabel();
  }

  private synccurrentCategoryLabel(): void {
    if (this.allActiveClass === 'active') {
      this.currentCategoryLabel = 'All';
      return;
    }

    const activeCategory = this.categories?.find((category) => category.activeClass === 'active');
    this.currentCategoryLabel = activeCategory?.title || 'All';
  }

  private onClick(event: MouseEvent): void {
    if (this.findPathElement(event, '.mobile-link')) {
      this.closeMobileDropdown();
    }
  }

  private closeMobileDropdown(): void {
    const dropdown = this.shadowRoot?.querySelector('.mobile-dropdown');
    if (dropdown instanceof HTMLDetailsElement) {
      dropdown.open = false;
    }
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

MpCategoryNav.defineAsync({
  name: 'mp-category-nav',
  templateOptions: 'defer-and-hydrate',
});
