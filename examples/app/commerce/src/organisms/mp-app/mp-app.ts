// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

import '#organisms/mp-navbar/mp-navbar.js';
import '#organisms/mp-mobile-menu/mp-mobile-menu.js';
import '#organisms/mp-category-nav/mp-category-nav.js';
import '#organisms/mp-cart-panel/mp-cart-panel.js';
import '#organisms/mp-footer/mp-footer.js';

export class MpApp extends RenderableFASTElement(FASTElement) {
  @attr({ attribute: 'store-name' }) storeName!: string;
  @attr({ attribute: 'cart-item-count' }) cartItemCount!: string;
  @attr({ attribute: 'search-query' }) searchQuery!: string;
  @attr({ attribute: 'current-path' }) currentPath!: string;
  @attr({ attribute: 'cart-open' }) cartOpen!: string;
  @attr({ attribute: 'cart-href' }) cartHref!: string;
  @attr({ attribute: 'cart-close-href' }) cartCloseHref!: string;
  @attr({ attribute: 'cart-subtotal' }) cartSubtotal!: string;
  @attr({ attribute: 'cart-taxes' }) cartTaxes!: string;
  @attr({ attribute: 'cart-empty' }) cartEmptyAttr!: string;
  @attr({ attribute: 'catalog-all-active-class' }) catalogAllActiveClass!: string;
  @attr({ attribute: 'catalog-current-label' }) catalogCurrentLabel!: string;
  @attr({ attribute: 'show-catalog-nav' }) showCatalogNav!: string;
  @attr({ attribute: 'shell-class' }) shellClass!: string;
  @attr page!: string;
  @observable catalogCategories?: any[];
  @observable cartItems?: any[];
  @observable cartEmpty!: boolean;

  private listenersAttached = false;
  private routeHandler = (e: Event): void => {
    const { routeName } = (e as CustomEvent).detail;
    this.searchQuery = this.searchQueryFromLocation();
    this.setPage(routeName);
    void this.syncShellChildren();
  };
  private catalogStateHandler = (e: Event): void => {
    const detail = (e as CustomEvent).detail as {
      categories?: unknown[];
      allActiveClass?: string;
    };

    if (Array.isArray(detail.categories)) {
      this.catalogCategories = detail.categories as any[];
    }
    if (detail.allActiveClass !== undefined) {
      this.catalogAllActiveClass = String(detail.allActiveClass);
    }
  };
  private cartStateHandler = (e: Event): void => {
    const detail = (e as CustomEvent).detail as Record<string, unknown>;
    this.applyCartState(detail);
    void this.syncShellChildren();
  };

  connectedCallback(): void {
    super.connectedCallback();
    if (this.listenersAttached) return;
    this.listenersAttached = true;
    window.addEventListener('webui:route:navigated', this.routeHandler);
    document.addEventListener('commerce:catalog-nav-state', this.catalogStateHandler);
    document.addEventListener('commerce:cart-state', this.cartStateHandler);
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    if (!this.listenersAttached) return;
    window.removeEventListener('webui:route:navigated', this.routeHandler);
    document.removeEventListener('commerce:catalog-nav-state', this.catalogStateHandler);
    document.removeEventListener('commerce:cart-state', this.cartStateHandler);
    this.listenersAttached = false;
  }

  setInitialState(state: Record<string, unknown>): void {
    if (state.storeName !== undefined) this.storeName = String(state.storeName);
    if (state.cartItemCount !== undefined) this.cartItemCount = String(state.cartItemCount);
    this.searchQuery = state.query !== undefined ? String(state.query) : '';
    this.applyCartState(state);
    if (state.page !== undefined) this.page = String(state.page);
    if (state.allActiveClass !== undefined) {
      this.catalogAllActiveClass = String(state.allActiveClass);
    }
    if (state.currentCategoryLabel !== undefined) {
      this.catalogCurrentLabel = String(state.currentCategoryLabel);
    }
    if (Array.isArray(state.categories)) {
      this.catalogCategories = state.categories as any[];
    }
    if (state.showCatalogNav !== undefined) {
      this.showCatalogNav = String(state.showCatalogNav);
    }
    if (state.shellClass !== undefined) {
      this.shellClass = String(state.shellClass);
    }
    this.setPage(this.page);
    void this.syncShellChildren();
  }

  async prepare(): Promise<void> {
    this.storeName = this.getAttribute('store-name') || 'Acme Store';
    this.cartItemCount = this.getAttribute('cart-item-count') || '0';
    this.searchQuery = this.getAttribute('search-query') || '';
    this.currentPath = this.getAttribute('current-path') || '/';
    this.cartOpen = this.getAttribute('cart-open') || '';
    this.cartHref = this.getAttribute('cart-href') || '/?cart=open';
    this.cartCloseHref = this.getAttribute('cart-close-href') || '/';
    this.cartSubtotal = this.getAttribute('cart-subtotal') || '$0.00';
    this.cartTaxes = this.getAttribute('cart-taxes') || '$0.00';
    this.cartEmptyAttr = this.getAttribute('cart-empty') || 'true';
    this.cartEmpty = this.cartEmptyAttr === 'true';
    this.page = this.getAttribute('page') || '';
    this.catalogAllActiveClass = this.getAttribute('catalog-all-active-class') || '';
    this.catalogCurrentLabel = this.getAttribute('catalog-current-label') || 'All';
    this.showCatalogNav = this.getAttribute('show-catalog-nav') || '';
    this.shellClass = this.getAttribute('shell-class') || 'default-shell';
    this.setPage(this.page || this.pageFromLocation());
  }

  private applyCartState(state: Record<string, unknown>): void {
    if (state.cartItemCount !== undefined) {
      this.cartItemCount = String(state.cartItemCount);
    }
    if (Array.isArray(state.cartItems)) {
      this.cartItems = state.cartItems as any[];
      this.cartEmpty = this.cartItems.length === 0;
      this.cartEmptyAttr = this.cartEmpty ? 'true' : 'false';
    }
    if (state.cartEmpty !== undefined) {
      this.cartEmpty = Boolean(state.cartEmpty);
      this.cartEmptyAttr = this.cartEmpty ? 'true' : 'false';
    }
    if (state.cartSubtotal !== undefined) {
      this.cartSubtotal = String(state.cartSubtotal);
    }
    if (state.cartTaxes !== undefined) {
      this.cartTaxes = String(state.cartTaxes);
    }
    if (state.currentPath !== undefined) {
      this.currentPath = String(state.currentPath);
    }
    if (state.cartOpen !== undefined) {
      this.cartOpen = String(state.cartOpen);
    }
    if (state.cartHref !== undefined) {
      this.cartHref = String(state.cartHref);
    }
    if (state.cartCloseHref !== undefined) {
      this.cartCloseHref = String(state.cartCloseHref);
    }
  }

  private async syncShellChildren(): Promise<void> {
    const sr = this.shadowRoot;
    if (!sr) return;

    await Promise.all([
      customElements.whenDefined('mp-navbar'),
      customElements.whenDefined('mp-mobile-menu'),
      customElements.whenDefined('mp-cart-panel'),
    ]);
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));

    this.pushAndRebind(sr, 'mp-navbar', {
      storeName: this.storeName,
      searchQuery: this.searchQuery,
      cartItemCount: this.cartItemCount,
      cartHref: this.cartHref,
    });
    this.pushAndRebind(sr, 'mp-mobile-menu', {
      searchQuery: this.searchQuery,
    });
    this.pushAndRebind(sr, 'mp-cart-panel', {
      cartItems: this.cartItems,
      cartEmpty: this.cartEmpty,
      subtotal: this.cartSubtotal,
      taxes: this.cartTaxes,
      cartOpen: this.cartOpen,
      cartCloseHref: this.cartCloseHref,
      currentPath: this.currentPath,
    });
  }

  private usesCatalogLayout(page: string): boolean {
    return page === 'search' || page === 'category';
  }

  private pageFromLocation(): string {
    const path = window.location.pathname;
    if (path === '/search') return 'search';
    if (path.startsWith('/search/')) return 'category';
    if (path.startsWith('/product/')) return 'product';
    if (path === '/') return 'home';
    return '';
  }

  private searchQueryFromLocation(): string {
    return new URLSearchParams(window.location.search).get('q') ?? '';
  }

  private setPage(page: string): void {
    this.page = page;
    const usesCatalogLayout = this.usesCatalogLayout(page);
    this.showCatalogNav = usesCatalogLayout ? 'true' : '';
    this.shellClass = usesCatalogLayout ? 'catalog-shell' : 'default-shell';
  }

  private pushAndRebind(
    sr: ShadowRoot,
    tag: string,
    data: Record<string, unknown>,
  ): void {
    const el = sr.querySelector(tag) as any;
    if (!el) return;
    for (const [key, value] of Object.entries(data)) {
      if (value !== undefined) {
        delete el[key];
        el[key] = value;
      }
    }
    const view = el.$fastController?.view;
    if (view) {
      view.unbind();
      view.bind(el, view.context);
    }
  }
}

MpApp.defineAsync({
  name: 'mp-app',
  templateOptions: 'defer-and-hydrate',
});
