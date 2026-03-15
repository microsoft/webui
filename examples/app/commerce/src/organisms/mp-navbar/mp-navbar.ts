import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

import '#atoms/mp-icon/mp-icon.js';
import '#molecules/mp-search-bar/mp-search-bar.js';

export class MpNavbar extends RenderableFASTElement(FASTElement) {
  @attr({ attribute: 'store-name' }) storeName!: string;
  @attr({ attribute: 'search-query' }) searchQuery!: string;
  @attr({ attribute: 'cart-item-count' }) cartItemCount!: string;
  @attr({ attribute: 'cart-href' }) cartHref = '/?cart=open';
  @observable navCategories?: { handle: string; title: string }[];
  private cartLinkHandler = (e: Event): void => {
    e.preventDefault();
    this.openCart();
  };

  private clickHandler = (e: Event): void => {
    if (this.findPathElement(e, '.cart-btn')) {
      e.preventDefault();
      this.openCart();
    } else if (this.findPathElement(e, '[data-action="open-menu"]')) {
      document.dispatchEvent(new CustomEvent('toggle-mobile-menu'));
    }
  };

  connectedCallback(): void {
    super.connectedCallback();
    this.addEventListener('click', this.clickHandler);
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    this.removeEventListener('click', this.clickHandler);
    this.shadowRoot?.querySelector('.cart-btn')?.removeEventListener('click', this.cartLinkHandler);
  }

  async prepare(): Promise<void> {
    this.storeName = this.getAttribute('store-name') || 'Acme Store';
    this.searchQuery = this.getAttribute('search-query') || '';
    this.cartItemCount = this.getAttribute('cart-item-count') || '0';
    this.cartHref = this.getAttribute('cart-href') || '/?cart=open';

    // Read categories from SSR'd nav links (skip "All" — it's hardcoded)
    const cats: { handle: string; title: string }[] = [];
    this.shadowRoot?.querySelectorAll('.nav-link').forEach(link => {
      const href = (link as HTMLAnchorElement).getAttribute('href') || '';
      const title = link.textContent?.trim() || '';
      if (title && href !== '/search') {
        // Category links are at /search/{handle}
        const handle = href.replace(/^\/search\//, '');
        cats.push({ handle, title });
      }
    });
    this.navCategories = cats;

    const cartLink = this.shadowRoot?.querySelector('.cart-btn');
    cartLink?.removeEventListener('click', this.cartLinkHandler);
    cartLink?.addEventListener('click', this.cartLinkHandler);
  }

  openCart(): void {
    document.dispatchEvent(new CustomEvent('toggle-cart'));
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

MpNavbar.defineAsync({
  name: 'mp-navbar',
  templateOptions: 'defer-and-hydrate',
});
