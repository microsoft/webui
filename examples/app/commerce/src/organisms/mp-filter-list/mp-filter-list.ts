import { FASTElement, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class MpFilterList extends RenderableFASTElement(FASTElement) {
  @observable sortOptions?: any[];
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
    if (Array.isArray(this.sortOptions)) {
      return;
    }

    // Data flows from parent via :sort-options binding.
    // On SSR hydration, read from SSR'd DOM as fallback.
    const sr = this.shadowRoot;
    if (!sr) return;
    const stateAttr = this.getAttribute('data-state');
    if (stateAttr) {
      try {
        const state = JSON.parse(stateAttr);
        if (Array.isArray(state.sortOptions)) {
          this.sortOptions = state.sortOptions;
          return;
        }
      } catch { /* ignore */ }
    }
    // Fallback: read from SSR'd DOM
    const links = sr.querySelectorAll('.desktop-list .link');
    if (links.length === 0) return;
    const options: any[] = [];
    links.forEach((link) => {
      const element = link as HTMLElement;
      options.push({
        value: element.getAttribute('data-value') || '',
        title: element.textContent?.trim() || '',
        href: element.getAttribute('data-href') || element.getAttribute('href') || '',
        activeClass: element.classList.contains('active') ? 'active' : '',
      });
    });
    this.sortOptions = options;
  }

  private onClick(event: MouseEvent): void {
    if (
      this.findPathElement(event, '.mobile-link')
      || this.findPathElement(event, '.mobile-current-item')
    ) {
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

MpFilterList.defineAsync({
  name: 'mp-filter-list',
  templateOptions: 'defer-and-hydrate',
});
