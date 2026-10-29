// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '@microsoft/webui-framework';
import { track } from './ntp-telemetry.js';

interface Market {
  code: string;
  language: string;
  region: string;
  native: string;
}

interface Row {
  code: string;
  label: string;
  native: string;
  active: boolean;
  current: boolean;
}

interface Recent {
  code: string;
  label: string;
}

const RECENT_KEY = 'ntp.recent-markets';
const RECENT_LIMIT = 4;
const PAGE = 8;
const MARKETS: readonly Market[] = [
  { code: 'en-US', language: 'English', region: 'United States', native: 'English (United States)' },
  { code: 'en-GB', language: 'English', region: 'United Kingdom', native: 'English (United Kingdom)' },
  { code: 'en-CA', language: 'English', region: 'Canada', native: 'English (Canada)' },
  { code: 'fr-CA', language: 'French', region: 'Canada', native: 'Fran\u00e7ais (Canada)' },
  { code: 'en-AU', language: 'English', region: 'Australia', native: 'English (Australia)' },
  { code: 'en-IN', language: 'English', region: 'India', native: 'English (India)' },
  { code: 'hi-IN', language: 'Hindi', region: 'India', native: '\u0939\u093f\u0928\u094d\u0926\u0940 (\u092d\u093e\u0930\u0924)' },
  { code: 'de-DE', language: 'German', region: 'Germany', native: 'Deutsch (Deutschland)' },
  { code: 'de-AT', language: 'German', region: 'Austria', native: 'Deutsch (\u00d6sterreich)' },
  { code: 'de-CH', language: 'German', region: 'Switzerland', native: 'Deutsch (Schweiz)' },
  { code: 'fr-FR', language: 'French', region: 'France', native: 'Fran\u00e7ais (France)' },
  { code: 'fr-BE', language: 'French', region: 'Belgium', native: 'Fran\u00e7ais (Belgique)' },
  { code: 'nl-NL', language: 'Dutch', region: 'Netherlands', native: 'Nederlands (Nederland)' },
  { code: 'nl-BE', language: 'Dutch', region: 'Belgium', native: 'Nederlands (Belgi\u00eb)' },
  { code: 'es-ES', language: 'Spanish', region: 'Spain', native: 'Espa\u00f1ol (Espa\u00f1a)' },
  { code: 'es-MX', language: 'Spanish', region: 'Mexico', native: 'Espa\u00f1ol (M\u00e9xico)' },
  { code: 'es-US', language: 'Spanish', region: 'United States', native: 'Espa\u00f1ol (Estados Unidos)' },
  { code: 'pt-BR', language: 'Portuguese', region: 'Brazil', native: 'Portugu\u00eas (Brasil)' },
  { code: 'pt-PT', language: 'Portuguese', region: 'Portugal', native: 'Portugu\u00eas (Portugal)' },
  { code: 'it-IT', language: 'Italian', region: 'Italy', native: 'Italiano (Italia)' },
  { code: 'sv-SE', language: 'Swedish', region: 'Sweden', native: 'Svenska (Sverige)' },
  { code: 'da-DK', language: 'Danish', region: 'Denmark', native: 'Dansk (Danmark)' },
  { code: 'nb-NO', language: 'Norwegian', region: 'Norway', native: 'Norsk bokm\u00e5l (Norge)' },
  { code: 'fi-FI', language: 'Finnish', region: 'Finland', native: 'Suomi (Suomi)' },
  { code: 'pl-PL', language: 'Polish', region: 'Poland', native: 'Polski (Polska)' },
  { code: 'cs-CZ', language: 'Czech', region: 'Czechia', native: '\u010ce\u0161tina (\u010cesko)' },
  { code: 'hu-HU', language: 'Hungarian', region: 'Hungary', native: 'Magyar (Magyarorsz\u00e1g)' },
  { code: 'tr-TR', language: 'Turkish', region: 'T\u00fcrkiye', native: 'T\u00fcrk\u00e7e (T\u00fcrkiye)' },
  { code: 'el-GR', language: 'Greek', region: 'Greece', native: '\u0395\u03bb\u03bb\u03b7\u03bd\u03b9\u03ba\u03ac (\u0395\u03bb\u03bb\u03ac\u03b4\u03b1)' },
  { code: 'ar-SA', language: 'Arabic', region: 'Saudi Arabia', native: '\u0627\u0644\u0639\u0631\u0628\u064a\u0629 (\u0627\u0644\u0633\u0639\u0648\u062f\u064a\u0629)' },
  { code: 'he-IL', language: 'Hebrew', region: 'Israel', native: '\u05e2\u05d1\u05e8\u05d9\u05ea (\u05d9\u05e9\u05e8\u05d0\u05dc)' },
  { code: 'ja-JP', language: 'Japanese', region: 'Japan', native: '\u65e5\u672c\u8a9e (\u65e5\u672c)' },
  { code: 'ko-KR', language: 'Korean', region: 'Korea', native: '\ud55c\uad6d\uc5b4 (\ub300\ud55c\ubbfc\uad6d)' },
  { code: 'zh-CN', language: 'Chinese (Simplified)', region: 'China', native: '\u4e2d\u6587\uff08\u4e2d\u56fd\uff09' },
  { code: 'zh-TW', language: 'Chinese (Traditional)', region: 'Taiwan', native: '\u4e2d\u6587\uff08\u53f0\u7063\uff09' },
  { code: 'zh-HK', language: 'Chinese (Traditional)', region: 'Hong Kong SAR', native: '\u4e2d\u6587\uff08\u9999\u6e2f\uff09' },
  { code: 'id-ID', language: 'Indonesian', region: 'Indonesia', native: 'Bahasa Indonesia (Indonesia)' },
  { code: 'th-TH', language: 'Thai', region: 'Thailand', native: '\u0e44\u0e17\u0e22 (\u0e44\u0e17\u0e22)' },
  { code: 'vi-VN', language: 'Vietnamese', region: 'Vietnam', native: 'Ti\u1ebfng Vi\u1ec7t (Vi\u1ec7t Nam)' },
  { code: 'en-ZA', language: 'English', region: 'South Africa', native: 'English (South Africa)' },
];

/** Lowercase and strip accents, so "espana" finds "Espa\u00f1a". */
function fold(text: string): string {
  return text.normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLowerCase();
}

function labelOf(market: Market): string {
  return market.language + ' (' + market.region + ')';
}

const INDEX = MARKETS.map(market => ({
  market,
  text: fold(market.language + ' ' + market.region + ' ' + market.native + ' ' + market.code),
}));

/** Every term must match; word starts rank above infix matches. */
function score(text: string, terms: readonly string[]): number {
  let total = 0;
  for (const term of terms) {
    const at = text.indexOf(term);
    if (at < 0) return 0;
    total += at === 0 ? 3 : text[at - 1] === ' ' || text[at - 1] === '(' ? 2 : 1;
  }
  return total;
}

/** Language and region picker with search, keyboard navigation, and recent choices. */
export class NtpMarketDialog extends WebUIElement {
  @attr current = 'en-US';
  @observable query = '';
  @observable results: readonly Row[] = [];
  @observable recent: readonly Recent[] = [];
  @observable detected = '';
  @observable status = '';
  @observable unchanged = true;

  searchInput!: HTMLInputElement;
  list!: HTMLElement;
  private choice = '';
  private active = 0;
  private found: readonly Market[] = MARKETS;

  protected override hydratedCallback(): void {
    this.choice = this.current;
    this.detected = this.detect();
    this.recent = this.readRecent();
    this.search('');
    requestAnimationFrame(() => this.searchInput?.focus({ preventScroll: true }));
    track('market.open', { current: this.current });
  }

  currentChanged(): void {
    this.choice = this.current;
    this.render();
  }

  onInput(event: Event): void {
    this.search((event.currentTarget as HTMLInputElement).value);
  }

  onKeydown(event: KeyboardEvent): void {
    switch (event.key) {
      case 'ArrowDown': this.move(1); break;
      case 'ArrowUp': this.move(-1); break;
      case 'PageDown': this.move(PAGE); break;
      case 'PageUp': this.move(-PAGE); break;
      case 'Enter': {
        const market = this.found[this.active];
        if (!market) return;
        this.choice = market.code;
        this.apply();
        break;
      }
      case 'Escape': this.dismiss(); break;
      default: return;
    }
    event.preventDefault();
  }

  choose(event: Event): void {
    const code = (event.currentTarget as HTMLElement).dataset['code'];
    if (!code) return;
    this.choice = code;
    this.active = Math.max(0, this.found.findIndex(market => market.code === code));
    this.render();
  }

  apply(): void {
    const market = MARKETS.find(entry => entry.code === this.choice);
    if (!market) return;
    this.writeRecent(market);
    track('market.apply', { from: this.current, to: market.code, query: this.query.length });
    this.$emit('commit', { code: market.code, label: labelOf(market) });
  }

  dismiss(): void {
    this.$emit('dismiss');
  }

  private search(query: string): void {
    this.query = query;
    const terms = fold(query).split(' ').filter(Boolean);
    if (terms.length === 0) {
      // The current and suggested markets lead an unfiltered list.
      const lead = new Set([this.choice, this.detected]);
      this.found = MARKETS.slice().sort((a, b) =>
        Number(lead.has(b.code) || lead.has(labelOf(b))) - Number(lead.has(a.code) || lead.has(labelOf(a))));
    } else {
      this.found = INDEX
        .map(entry => ({ market: entry.market, score: score(entry.text, terms) }))
        .filter(entry => entry.score > 0)
        .sort((a, b) => b.score - a.score || labelOf(a.market).localeCompare(labelOf(b.market)))
        .map(entry => entry.market);
    }
    this.active = 0;
    this.status = this.found.length === 0 ? 'No languages or regions match "' + query + '".'
      : this.found.length + (this.found.length === 1 ? ' result' : ' results');
    this.render();
  }

  private move(step: number): void {
    if (this.found.length === 0) return;
    this.active = Math.min(this.found.length - 1, Math.max(0, this.active + step));
    this.render();
    this.list.children[this.active]?.scrollIntoView({ block: 'nearest' });
  }

  private render(): void {
    this.results = this.found.map((market, index) => ({
      code: market.code,
      label: labelOf(market),
      native: market.native,
      active: index === this.active,
      current: market.code === this.choice,
    }));
    this.unchanged = this.choice === this.current;
  }

  /** The best market for the browser's preferred languages. */
  private detect(): string {
    const languages = navigator.languages?.length ? navigator.languages : [navigator.language];
    for (const language of languages) {
      const lower = language.toLowerCase();
      const exact = MARKETS.find(market => market.code.toLowerCase() === lower);
      if (exact) return labelOf(exact);
      const base = lower.split('-')[0];
      const partial = MARKETS.find(market => market.code.toLowerCase().startsWith(base + '-'));
      if (partial) return labelOf(partial);
    }
    return labelOf(MARKETS[0]);
  }

  private readRecent(): Recent[] {
    try {
      const codes: unknown = JSON.parse(localStorage.getItem(RECENT_KEY) ?? '[]');
      if (!Array.isArray(codes)) return [];
      const recent: Recent[] = [];
      for (const code of codes) {
        const market = MARKETS.find(entry => entry.code === code);
        if (market && market.code !== this.current) recent.push({ code: market.code, label: labelOf(market) });
      }
      return recent.slice(0, RECENT_LIMIT);
    } catch {
      return [];
    }
  }

  private writeRecent(market: Market): void {
    try {
      const codes = [this.current, ...this.recent.map(entry => entry.code)]
        .filter(code => code !== market.code)
        .slice(0, RECENT_LIMIT);
      localStorage.setItem(RECENT_KEY, JSON.stringify(codes));
    } catch {
      // Storage can be full or disabled; recent markets are a convenience.
    }
  }
}

NtpMarketDialog.define('ntp-market-dialog');
