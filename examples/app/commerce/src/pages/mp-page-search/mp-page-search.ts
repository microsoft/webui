import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

import '#organisms/mp-product-grid/mp-product-grid.js';
import '#organisms/mp-filter-list/mp-filter-list.js';

export class MpPageSearch extends RenderableFASTElement(FASTElement) {
  @observable products?: any[];
  @observable categories?: any[];
  @observable sortOptions?: any[];
  @observable query!: string;
  @observable resultsCount!: number;
  @attr({ attribute: 'all-active-class' }) allActiveClass = '';

  async prepare(): Promise<void> {
    const raw = this.getAttribute('data-state');
    if (raw) {
      try {
        this.applyState(JSON.parse(raw));
        this.emitCatalogNavState();
      } catch { /* ignore parse errors */ }
    }

    if (
      this.products !== undefined
      || this.categories !== undefined
      || this.sortOptions !== undefined
    ) {
      // FAST can bind child views before their DOM-extraction fallback runs.
      // Push the page state back into children during initial hydration too.
      await this.syncChildren();
    }
  }

  setInitialState(
    state: Record<string, unknown>,
    params?: Record<string, string>,
  ): void {
    this.applyState(state, params);
    this.emitCatalogNavState();
    // SPA navigation: push data to children and rebind their views
    this.syncChildren();
  }

  private applyState(
    state: Record<string, unknown>,
    params?: Record<string, string>,
  ): void {
    if (Array.isArray(state.products)) {
      this.products = state.products as any[];
    }
    if (Array.isArray(state.categories)) {
      this.categories = state.categories as any[];
    }
    if (Array.isArray(state.sortOptions)) {
      this.sortOptions = state.sortOptions as any[];
    }
    if (state.query !== undefined) {
      this.query = String(state.query);
    }
    if (typeof state.resultsCount === 'number') {
      this.resultsCount = state.resultsCount;
    }
    if (params?.category) {
      this.allActiveClass = '';
    } else {
      this.allActiveClass = String(state.allActiveClass ?? 'active');
    }
  }

  private async syncChildren(): Promise<void> {
    const sr = this.shadowRoot;
    if (!sr) return;

    await Promise.all([
      customElements.whenDefined('mp-product-grid'),
      customElements.whenDefined('mp-filter-list'),
    ]);
    await new Promise<void>(r => requestAnimationFrame(() => r()));

    this.pushAndRebind(sr, 'mp-product-grid', { products: this.products });
    this.pushAndRebind(sr, 'mp-filter-list', { sortOptions: this.sortOptions });
  }

  private emitCatalogNavState(): void {
    document.dispatchEvent(new CustomEvent('commerce:catalog-nav-state', {
      detail: {
        categories: this.categories,
        allActiveClass: this.allActiveClass,
      },
    }));
  }

  /** Set observable properties on a child and rebind its FAST view. */
  private pushAndRebind(
    sr: ShadowRoot,
    tag: string,
    data: Record<string, unknown>,
  ): void {
    const el = sr.querySelector(tag) as any;
    if (!el) return;
    let updated = false;
    for (const [key, value] of Object.entries(data)) {
      if (value !== undefined) {
        delete el[key]; // remove any shadowing instance property
        el[key] = value;
        updated = true;
      }
    }
    if (!updated) return;
    const view = el.$fastController?.view;
    if (view) {
      view.unbind();
      view.bind(el, view.context);
    }
  }
}

MpPageSearch.defineAsync({
  name: 'mp-page-search',
  templateOptions: 'defer-and-hydrate',
});
