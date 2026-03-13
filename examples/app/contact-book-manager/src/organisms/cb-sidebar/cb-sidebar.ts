import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class CbSidebar extends RenderableFASTElement(FASTElement) {
  @attr page = 'dashboard';
  @attr({ attribute: 'active-group' }) activeGroup = 'all';
  @attr({ attribute: 'total-contacts' }) totalContacts = '0';
  @attr({ attribute: 'total-favorites' }) totalFavorites = '0';
  groups!: string[];

  async prepare(): Promise<void> {
    if (!this.shadowRoot) return;

    const groups: string[] = [];
    for (const el of this.shadowRoot.querySelectorAll('.nav-item')) {
      const label = el.getAttribute('data-nav') || '';
      if (['Dashboard', 'All Contacts', 'Favorites'].includes(label)) continue;
      if (label) groups.push(label);
    }
    this.groups = groups;
  }

  pageChanged(): void {
    this.updateActiveState();
  }

  activeGroupChanged(): void {
    this.updateActiveState();
  }

  /** Sync data-active attributes to match current page/group. */
  private updateActiveState(): void {
    for (const el of this.shadowRoot?.querySelectorAll('.nav-item') || []) {
      const nav = el.getAttribute('data-nav') || '';
      const p = this.page;
      let isActive = false;
      if (nav === 'Dashboard') isActive = p === 'dashboard';
      else if (nav === 'All Contacts') isActive = p === 'contacts' || p === 'detail' || p === 'add' || p === 'edit';
      else if (nav === 'Favorites') isActive = p === 'favorites';
      else if (p === 'group') isActive = nav === this.activeGroup;
      el.toggleAttribute('data-active', isActive);
    }
  }
}

CbSidebar.defineAsync({
  name: 'cb-sidebar',
  templateOptions: 'defer-and-hydrate',
});
