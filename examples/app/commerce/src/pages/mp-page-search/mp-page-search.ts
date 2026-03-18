// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

import '#organisms/mp-filter-list/mp-filter-list.js';

function waitForView(el: any, maxFrames = 10): Promise<void> {
  return new Promise<void>((resolve) => {
    let frame = 0;
    const check = (): void => {
      if (el.$fastController?.view) { resolve(); return; }
      if (++frame >= maxFrames) { resolve(); return; }
      requestAnimationFrame(check);
    };
    check();
  });
}

function waitForChild(sr: ShadowRoot, selector: string, maxFrames = 20): Promise<HTMLElement | null> {
  return new Promise<HTMLElement | null>((resolve) => {
    const el = sr.querySelector(selector) as HTMLElement | null;
    if (el) { resolve(el); return; }
    let frame = 0;
    const check = (): void => {
      const found = sr.querySelector(selector) as HTMLElement | null;
      if (found) { resolve(found); return; }
      if (++frame >= maxFrames) { resolve(null); return; }
      requestAnimationFrame(check);
    };
    requestAnimationFrame(check);
  });
}

export class MpPageSearch extends RenderableFASTElement(FASTElement) {
  @observable categories?: any[];
  @observable sortOptions?: any[];
  @attr({ attribute: 'all-active-class' }) allActiveClass = '';
  @attr({ attribute: 'current-label' }) currentCategoryLabel = 'All';

  async prepare(): Promise<void> {
    const raw = this.getAttribute('data-state');
    if (raw) {
      try {
        const state = JSON.parse(raw);
        this.applyState(state);
        this.emitCatalogNavState();
        await this.initChildren(state);
      } catch { /* ignore parse errors */ }
    }
  }

  setInitialState(
    state: Record<string, unknown>,
    params?: Record<string, string>,
  ): void {
    this.applyState(state, params);
    this.emitCatalogNavState();
    void this.initChildren(state);
  }

  private applyState(
    state: Record<string, unknown>,
    params?: Record<string, string>,
  ): void {
    if (Array.isArray(state.categories)) {
      this.categories = state.categories as any[];
    }
    if (Array.isArray(state.sortOptions)) {
      this.sortOptions = state.sortOptions as any[];
    }
    if (params?.category) {
      this.allActiveClass = '';
    } else {
      this.allActiveClass = String(state.allActiveClass ?? 'active');
    }
    if (state.currentCategoryLabel !== undefined) {
      this.currentCategoryLabel = String(state.currentCategoryLabel);
    }
  }

  private async initChildren(state: Record<string, unknown>): Promise<void> {
    const sr = this.shadowRoot;
    if (!sr) return;

    await Promise.all([
      customElements.whenDefined('mp-category-nav'),
      customElements.whenDefined('mp-filter-list'),
    ]);

    const catNav = await waitForChild(sr, 'mp-category-nav');
    if (catNav) {
      await waitForView(catNav);
      (catNav as any).setInitialState?.(state);
    }

    const filterList = await waitForChild(sr, 'mp-filter-list');
    if (filterList) {
      await waitForView(filterList);
      (filterList as any).setInitialState?.(state);
    }
  }

  private emitCatalogNavState(): void {
    document.dispatchEvent(new CustomEvent('commerce:catalog-nav-state', {
      detail: {
        categories: this.categories,
        allActiveClass: this.allActiveClass,
      },
    }));
  }
}

MpPageSearch.defineAsync({
  name: 'mp-page-search',
  templateOptions: 'defer-and-hydrate',
});
