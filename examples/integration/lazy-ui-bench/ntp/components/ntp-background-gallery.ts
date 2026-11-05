// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';
import { track } from './ntp-telemetry.js';

/** One background image: identity, title, gradient stops, and credit. */
type ImageRow = readonly [id: string, title: string, from: string, to: string, credit: string];

interface Collection {
  id: string;
  label: string;
  images: readonly ImageRow[];
}

interface Swatch {
  id: string;
  label: string;
  color: string;
}

interface Choice {
  collection: string;
  image: string;
  daily: boolean;
}

const STORAGE_KEY = 'ntp.background';
const DAY_MS = 86_400_000;
const COLLECTIONS: readonly Collection[] = [
  { id: 'nature', label: 'Nature', images: [
    ['rainier', 'Mount Rainier at sunrise', '#f6a96b', '#3b4f7a', 'Kenji Watanabe'],
    ['fjord', 'Geirangerfjord, Norway', '#9cc7d9', '#25485e', 'Ingrid Solberg'],
    ['dunes', 'Erg Chebbi dunes, Morocco', '#f3c27a', '#a2522a', 'Amina Idrissi'],
    ['aurora', 'Aurora over Abisko', '#55e3a3', '#0b1f3a', 'Lars Nyberg'],
    ['redwoods', 'Redwood National Park', '#a6c48a', '#2f3b1c', 'Maya Ortiz'],
    ['lavender', 'Lavender fields, Valensole', '#c9a7e8', '#5b3d7a', 'Claire Fabre'],
    ['glacier', 'Perito Moreno Glacier', '#cfe8f5', '#4b7da3', 'Diego Herrera'],
    ['falls', 'Sk\u00f3gafoss, Iceland', '#d7e3e8', '#3d5a52', 'Sigr\u00fan Bj\u00f6rk'],
  ] },
  { id: 'cities', label: 'Cities', images: [
    ['seattle', 'Seattle skyline at dusk', '#f08a5d', '#2c2c54', 'Jordan Lee'],
    ['tokyo', 'Shibuya crossing at night', '#ff5f9e', '#1b1340', 'Aiko Tanaka'],
    ['lisbon', 'Alfama rooftops, Lisbon', '#f7b267', '#7a3e48', 'Rui Costa'],
    ['nyc', 'Brooklyn Bridge in fog', '#c4ccd6', '#3a4252', 'Sam Okafor'],
    ['dubai', 'Dubai Marina at blue hour', '#4f9de8', '#151d3b', 'Omar Haddad'],
    ['prague', 'Charles Bridge, Prague', '#e8c07d', '#4a3b32', 'Petra Nov\u00e1kov\u00e1'],
    ['singapore', 'Gardens by the Bay', '#7bd389', '#123047', 'Wei Ling Tan'],
    ['sydney', 'Sydney Harbour', '#5bc0eb', '#1d3557', 'Chloe Martin'],
  ] },
  { id: 'abstract', label: 'Abstract', images: [
    ['flow', 'Flow', '#ff9a8b', '#6a4c93', 'Studio Lumen'],
    ['prism', 'Prism', '#43cbff', '#9708cc', 'Studio Lumen'],
    ['ember', 'Ember', '#ffb347', '#c0392b', 'Studio Lumen'],
    ['tide', 'Tide', '#a1ffce', '#1d976c', 'Studio Lumen'],
    ['dusk', 'Dusk', '#fbc2eb', '#4a3f8a', 'Studio Lumen'],
    ['graphite', 'Graphite', '#bdc3c7', '#2c3e50', 'Studio Lumen'],
    ['citrus', 'Citrus', '#f6d365', '#fda085', 'Studio Lumen'],
    ['lagoon', 'Lagoon', '#2af598', '#009efd', 'Studio Lumen'],
  ] },
  { id: 'space', label: 'Space', images: [
    ['pillars', 'Pillars of Creation', '#c58b5a', '#1a1023', 'NASA, ESA, CSA'],
    ['andromeda', 'Andromeda Galaxy', '#a3a1d6', '#0d0b26', 'Rafael Mendes'],
    ['earthrise', 'Earthrise', '#5aa0d6', '#05070f', 'NASA'],
    ['nebula', 'Carina Nebula', '#f28c6b', '#14102a', 'NASA, ESA, CSA'],
    ['milkyway', 'Milky Way over Atacama', '#7b6fd6', '#0a0a1f', 'Valentina Rojas'],
    ['saturn', 'Saturn and its rings', '#e6cf9b', '#16130d', 'NASA, JPL'],
    ['moon', 'Waxing gibbous moon', '#d9d9d9', '#121212', 'Arjun Mehta'],
    ['eclipse', 'Total solar eclipse', '#ffd27f', '#000000', 'Hannah Becker'],
  ] },
];

function rgb(hex: string): [number, number, number] {
  const value = parseInt(hex.slice(1), 16);
  return [(value >> 16) & 255, (value >> 8) & 255, value & 255];
}

function toHex(channels: readonly number[]): string {
  return '#' + channels.map(channel => Math.round(channel).toString(16).padStart(2, '0')).join('');
}

function mix(from: string, to: string, amount: number): string {
  const a = rgb(from);
  const b = rgb(to);
  return toHex(a.map((channel, i) => channel + (b[i] - channel) * amount));
}

/** WCAG relative luminance. */
function luminance(hex: string): number {
  const [r, g, b] = rgb(hex).map(channel => {
    const value = channel / 255;
    return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** Text color with the higher contrast against the image's middle tone. */
function inkFor(from: string, to: string): string {
  return luminance(mix(from, to, 0.5)) > 0.36 ? '#1b1b1b' : '#ffffff';
}

function dayOfYear(date: Date): number {
  return Math.floor((date.getTime() - new Date(date.getFullYear(), 0, 1).getTime()) / DAY_MS);
}

function find(collection: string): Collection {
  return COLLECTIONS.find(entry => entry.id === collection) ?? COLLECTIONS[0];
}

/** Background gallery: collections, preview, daily refresh, and a shared swatch grid. */
export class NtpBackgroundGallery extends WebUIElement {
  @observable tabs = COLLECTIONS.map(collection => ({ id: collection.id, label: collection.label }));
  @observable category = COLLECTIONS[0].id;
  @observable swatches: readonly Swatch[] = [];
  @observable selected = '';
  @observable imageTitle = '';
  @observable imageCredit = '';
  @observable preview = '';
  @observable ink = '#ffffff';
  @observable daily = true;
  @observable status = '';

  protected override hydratedCallback(): void {
    const saved = this.read();
    this.daily = saved.daily;
    this.show(saved.collection, saved.daily ? this.dailyImage(saved.collection) : saved.image);
    track('gallery.open', { collection: this.category });
  }

  selectCategory(event: Event): void {
    const id = (event.currentTarget as HTMLElement).dataset['id'];
    if (id && id !== this.category) this.show(id, find(id).images[0][0]);
  }

  /** Left and right arrows move between collection tabs. */
  onTabKey(event: KeyboardEvent): void {
    const step = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0;
    if (!step) return;
    event.preventDefault();
    const index = COLLECTIONS.findIndex(collection => collection.id === this.category);
    const next = COLLECTIONS[(index + step + COLLECTIONS.length) % COLLECTIONS.length];
    this.show(next.id, next.images[0][0]);
    this.shadowRoot?.querySelector<HTMLElement>('.tab[data-id="' + next.id + '"]')?.focus();
  }

  chooseImage(event: CustomEvent<{ id: string }>): void {
    this.show(this.category, event.detail.id);
    this.status = this.imageTitle + ' selected';
  }

  toggleDaily(event: Event): void {
    this.daily = (event.currentTarget as HTMLInputElement).checked;
    if (this.daily) this.show(this.category, this.dailyImage(this.category));
    this.status = this.daily ? 'A new image will appear every day.' : 'This image will stay until you change it.';
  }

  /** Pick a different image, avoiding the current one. */
  shuffle(): void {
    const images = COLLECTIONS.flatMap(collection => collection.images.map(image => [collection.id, image[0]] as const));
    const options = images.filter(([, id]) => id !== this.selected);
    const [collection, image] = options[Math.floor(Math.random() * options.length)];
    this.daily = false;
    this.show(collection, image);
    this.status = 'Showing ' + this.imageTitle;
  }

  onKeydown(event: KeyboardEvent): void {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    this.dismiss();
  }

  apply(): void {
    const choice: Choice = { collection: this.category, image: this.selected, daily: this.daily };
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(choice));
    } catch {
      this.status = 'This image applies now but could not be saved.';
    }
    track('gallery.apply', { ...choice });
    this.$emit('commit', { title: this.imageTitle, credit: this.imageCredit, gradient: this.preview });
  }

  dismiss(): void {
    this.$emit('dismiss');
  }

  private show(collectionId: string, imageId: string): void {
    const collection = find(collectionId);
    const image = collection.images.find(entry => entry[0] === imageId) ?? collection.images[0];
    const [id, title, from, to, credit] = image;
    this.category = collection.id;
    this.swatches = collection.images.map(([swatchId, label, start, end]) => ({
      id: swatchId,
      label,
      color: 'linear-gradient(160deg, ' + start + ', ' + end + ')',
    }));
    this.selected = id;
    this.imageTitle = title;
    this.imageCredit = 'Photo: ' + credit;
    this.preview = 'linear-gradient(160deg, ' + from + ' 0%, ' + mix(from, to, 0.45) + ' 55%, ' + to + ' 100%)';
    this.ink = inkFor(from, to);
  }

  private dailyImage(collectionId: string): string {
    const images = find(collectionId).images;
    return images[dayOfYear(new Date()) % images.length][0];
  }

  private read(): Choice {
    try {
      const value: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? 'null');
      if (value && typeof value === 'object') {
        const choice = value as Partial<Choice>;
        const collection = find(String(choice.collection ?? ''));
        const image = collection.images.some(entry => entry[0] === choice.image) ? String(choice.image) : collection.images[0][0];
        return { collection: collection.id, image, daily: choice.daily !== false };
      }
    } catch {
      // Fall through to the default collection.
    }
    return { collection: COLLECTIONS[0].id, image: COLLECTIONS[0].images[0][0], daily: true };
  }
}

NtpBackgroundGallery.define('ntp-background-gallery');
