// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

interface AppMeta {
  slug: string;
  name: string;
  description: string;
  backend: string;
  sourceUrl: string;
  iframeUrl: string;
}

export class DemoShell extends WebUIElement {
  @observable apps: AppMeta[] = [];
  @observable currentApp: AppMeta = {
    slug: '',
    name: '',
    description: '',
    backend: '',
    sourceUrl: '',
    iframeUrl: '',
  };
  @observable currentDisplay = 1;
  @observable totalApps = 0;

  selectEl!: HTMLSelectElement;
  frameEl!: HTMLIFrameElement;

  private currentIndex = 0;

  connectedCallback(): void {
    super.connectedCallback();
    this.currentIndex = Math.max(
      0,
      this.apps.findIndex(a => a.slug === this.currentApp.slug),
    );
    window.addEventListener('keydown', this.onKeydown);
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    window.removeEventListener('keydown', this.onKeydown);
  }

  onPrev(): void {
    this.navigate(this.currentIndex - 1);
  }

  onNext(): void {
    this.navigate(this.currentIndex + 1);
  }

  onSelectChange(e: Event): void {
    const slug = (e.target as HTMLSelectElement).value;
    const idx = this.apps.findIndex(a => a.slug === slug);
    if (idx >= 0) this.navigate(idx);
  }

  private onKeydown = (e: KeyboardEvent): void => {
    if (e.target instanceof HTMLElement) {
      const tag = e.target.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;
    }
    if (e.key === 'ArrowLeft') this.onPrev();
    else if (e.key === 'ArrowRight') this.onNext();
  };

  private navigate(idx: number): void {
    if (idx < 0 || idx >= this.apps.length || idx === this.currentIndex) return;
    this.currentIndex = idx;
    const app = this.apps[idx];

    this.swapFrame(app);

    this.currentApp = app;
    this.currentDisplay = idx + 1;
    if (this.selectEl) this.selectEl.value = app.slug;

    const url = new URL(window.location.href);
    url.searchParams.set('app', app.slug);
    history.replaceState(null, '', url.toString());
    document.title = `${app.name} — WebUI Demo`;
  }

  // Replace the iframe with a fresh element. The hosted apps install a
  // Navigation-API listener (webui-router) that intercepts every navigation
  // in their frame, including external `iframe.src` changes — leaving the
  // iframe stuck on the previous document. A fresh iframe has no router
  // installed yet, so the navigation completes normally.
  private swapFrame(app: AppMeta): void {
    if (!this.frameEl) return;
    const fresh = document.createElement('iframe');
    fresh.className = this.frameEl.className;
    fresh.title = app.name;
    fresh.src = app.iframeUrl;
    this.frameEl.replaceWith(fresh);
    this.frameEl = fresh;
  }
}

DemoShell.define('demo-shell');
