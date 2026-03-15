import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

import '#atoms/mp-icon/mp-icon.js';
import '#atoms/mp-price/mp-price.js';
import '#atoms/mp-product-image/mp-product-image.js';

interface CartItem {
  handle: string;
  title: string;
  color: string;
  size: string;
  variantLabel: string;
  price: string;
  quantity: number;
  gradient: string;
  imageUrl: string;
  increaseTo: number;
  decreaseTo: number;
  redirectTo: string;
}

export class MpCartPanel extends RenderableFASTElement(FASTElement) {
  @observable cartItems?: CartItem[];
  @observable cartEmpty!: boolean;
  @attr subtotal!: string;
  @attr taxes!: string;
  @attr({ attribute: 'cart-open' }) cartOpen!: string;
  @attr({ attribute: 'cart-close-href' }) cartCloseHref!: string;
  @attr({ attribute: 'current-path' }) currentPath!: string;
  @observable panelOpenClass!: string;
  @observable backdropOpenClass!: string;

  private toggleHandler = (): void => { this.openCart(); };
  private shadowClickHandler = (e: Event): void => { this.onShadowClick(e as MouseEvent); };

  connectedCallback(): void {
    super.connectedCallback();
    document.addEventListener('toggle-cart', this.toggleHandler);
    this.addEventListener('click', this.shadowClickHandler);
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    document.removeEventListener('toggle-cart', this.toggleHandler);
    this.removeEventListener('click', this.shadowClickHandler);
  }

  async prepare(): Promise<void> {
    this.cartOpen = this.getAttribute('cart-open') || '';
    this.cartCloseHref = this.getAttribute('cart-close-href') || '/';
    this.currentPath = this.getAttribute('current-path') || '/';
    this.subtotal = this.getAttribute('subtotal') || '$0.00';
    this.taxes = this.getAttribute('taxes') || '$0.00';

    if (Array.isArray(this.cartItems)) {
      this.cartEmpty = this.cartItems.length === 0;
      this.syncOpenState();
      return;
    }

    const items: CartItem[] = [];
    this.shadowRoot!.querySelectorAll('.cart-line').forEach(line => {
      const image = line.querySelector('mp-product-image') as HTMLElement | null;
      const title = line.querySelector('.item-title') as HTMLAnchorElement;
      const variant = line.querySelector('.item-variant')?.textContent?.trim() || '';
      const price = line.querySelector('mp-price')?.getAttribute('value') || '';
      const qty = line.querySelector('.qty-count')?.textContent?.trim() || '1';
      const btn = line.querySelector('[data-handle]') as HTMLElement;

      const parts = variant.split('/').map(s => s.trim());
      items.push({
        handle: btn?.getAttribute('data-handle') || '',
        title: title?.textContent?.trim() || '',
        color: parts[0] || '',
        size: parts[1] || '',
        variantLabel: variant,
        price,
        quantity: parseInt(qty, 10) || 1,
        gradient: image?.getAttribute('gradient') || '',
        imageUrl: image?.getAttribute('image-url') || '',
        increaseTo: parseInt(btn?.getAttribute('data-quantity') || '2', 10) || 2,
        decreaseTo: 0,
        redirectTo: btn?.closest('form')?.querySelector<HTMLInputElement>('input[name="redirectTo"]')?.value || this.currentPath,
      });
    });

    this.cartItems = items;
    this.cartEmpty = items.length === 0;
    this.syncOpenState();
  }

  cartItemsChanged(): void {
    if (Array.isArray(this.cartItems)) {
      this.cartEmpty = this.cartItems.length === 0;
    }
  }

  cartOpenChanged(): void {
    this.syncOpenState();
  }

  onShadowClick(e: MouseEvent): void {
    if (this.findPathElement(e, '[data-action="close"]') || this.findPathElement(e, '.backdrop')) {
      e.preventDefault();
      this.closeCart();
      return;
    }

    const btn = this.findPathElement(
      e,
      '[data-action="increase"], [data-action="decrease"], [data-action="remove"]',
    ) as HTMLElement | null;
    if (btn) {
      e.preventDefault();
      void this.handleQuantity(btn);
      return;
    }
  }

  async handleQuantity(btn: HTMLElement): Promise<void> {
    const handle = btn.getAttribute('data-handle') || '';
    const color = btn.getAttribute('data-color') || '';
    const size = btn.getAttribute('data-size') || '';
    const quantity = parseInt(btn.getAttribute('data-quantity') || '0', 10);
    if (!handle || Number.isNaN(quantity)) {
      return;
    }

    await this.submitCartMutation('/cart/update', {
      handle,
      color,
      size,
      quantity,
      redirectTo: this.currentPath,
      openCart: true,
    });
  }

  openCart(): void {
    this.cartOpen = 'true';
    this.syncOpenState();
  }

  closeCart(): void {
    this.cartOpen = '';
    this.syncOpenState();
  }

  private syncOpenState(): void {
    const isOpen = this.cartOpen === 'true';
    this.panelOpenClass = isOpen ? 'is-open' : '';
    this.backdropOpenClass = isOpen ? 'open' : '';
    const html = document.documentElement;
    const { body } = document;

    html.style.overflow = isOpen ? 'hidden' : '';
    html.style.scrollbarGutter = isOpen ? 'stable' : '';
    html.style.overscrollBehavior = isOpen ? 'none' : '';

    body.style.overflow = isOpen ? 'hidden' : '';
    body.style.scrollbarGutter = isOpen ? 'stable' : '';
    body.style.overscrollBehavior = isOpen ? 'none' : '';
  }

  private async submitCartMutation(url: string, payload: Record<string, unknown>): Promise<void> {
    const response = await fetch(url, {
      method: 'POST',
      headers: {
        Accept: 'application/json',
        'Content-Type': 'application/json',
      },
      body: JSON.stringify(payload),
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

MpCartPanel.defineAsync({
  name: 'mp-cart-panel',
  templateOptions: 'defer-and-hydrate',
});
