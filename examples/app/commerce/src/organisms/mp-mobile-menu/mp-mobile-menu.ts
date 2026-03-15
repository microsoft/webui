import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

import '#atoms/mp-icon/mp-icon.js';
import '#molecules/mp-search-bar/mp-search-bar.js';

interface Category {
  handle: string;
  title: string;
}

export class MpMobileMenu extends RenderableFASTElement(FASTElement) {
  @attr({ attribute: 'search-query' }) searchQuery = '';
  @observable navCategories?: Category[];

  private clickHandler = (e: Event): void => { this.onClick(e as MouseEvent); };
  private toggleHandler = (): void => { this.openMenu(); };
  private resizeHandler = (): void => {
    if (window.innerWidth >= 768) {
      this.closeMenu();
    }
  };

  connectedCallback(): void {
    super.connectedCallback();
    this.addEventListener('click', this.clickHandler);
    document.addEventListener('toggle-mobile-menu', this.toggleHandler);
    window.addEventListener('resize', this.resizeHandler);
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    this.removeEventListener('click', this.clickHandler);
    document.removeEventListener('toggle-mobile-menu', this.toggleHandler);
    window.removeEventListener('resize', this.resizeHandler);
  }

  private get panelEl(): HTMLElement | null {
    return this.shadowRoot?.querySelector('#mobile-menu') ?? null;
  }

  private get backdropEl(): HTMLElement | null {
    return this.shadowRoot?.querySelector('.backdrop') ?? null;
  }

  async prepare(): Promise<void> {
    this.searchQuery = this.getAttribute('search-query') || '';
    const cats: Category[] = [];
    this.shadowRoot!.querySelectorAll('.menu-link').forEach(link => {
      const href = link.getAttribute('href') || '';
      const title = link.textContent?.trim() || '';
      if (href !== '/search' && title) {
        const handle = href.replace('/search/', '');
        cats.push({ handle, title });
      }
    });
    this.navCategories = cats;
  }

  onClick(e: MouseEvent): void {
    if (this.findPathElement(e, '.menu-link')) {
      this.closeMenu();
      return;
    }

    const action = this.findPathElement(e, '[data-action]')?.getAttribute('data-action');
    if (action === 'close') {
      this.closeMenu();
    }
  }

  private openMenu(): void {
    this.panelEl?.showPopover();
    this.backdropEl?.classList.add('open');
  }

  private closeMenu(): void {
    this.panelEl?.hidePopover();
    this.backdropEl?.classList.remove('open');
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

MpMobileMenu.defineAsync({
  name: 'mp-mobile-menu',
  templateOptions: 'defer-and-hydrate',
});
