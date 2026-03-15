import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

function normalize_loading(value: string | null): string {
  return value === 'eager' || value === 'auto' ? value : 'lazy';
}

function normalize_decoding(value: string | null): string {
  return value === 'sync' || value === 'auto' ? value : 'async';
}

function normalize_fetch_priority(value: string | null): string {
  return value === 'high' || value === 'low' ? value : 'auto';
}

export class MpProductImage extends RenderableFASTElement(FASTElement) {
  @attr gradient!: string;
  @attr({ attribute: 'image-url' }) imageUrl!: string;
  @attr alt!: string;
  @attr interactive!: string;
  @attr loading!: string;
  @attr decoding!: string;
  @attr({ attribute: 'fetch-priority' }) fetchPriority!: string;

  async prepare(): Promise<void> {
    this.gradient = this.getAttribute('gradient') || '';
    this.imageUrl = this.getAttribute('image-url') || '';
    this.alt = this.getAttribute('alt') || '';
    this.interactive = this.getAttribute('interactive') || '';
    this.loading = normalize_loading(this.getAttribute('loading'));
    this.decoding = normalize_decoding(this.getAttribute('decoding'));
    this.fetchPriority = normalize_fetch_priority(this.getAttribute('fetch-priority'));
  }
}

MpProductImage.defineAsync({
  name: 'mp-product-image',
  templateOptions: 'defer-and-hydrate',
});
