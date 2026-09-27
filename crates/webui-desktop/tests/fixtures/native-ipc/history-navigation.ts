// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

const CANCELLATION_KEY = 'native-ipc-history-cancellation';

interface EvidenceStore {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

interface Page {
  addEventListener(name: 'pagehide', listener: (event: { isTrusted: boolean }) => void): void;
  removeEventListener(name: 'pagehide', listener: (event: { isTrusted: boolean }) => void): void;
}

export class HistoryNavigationHold {
  private navigating = false;
  private transportFailed = false;
  private hidden = false;

  constructor(private phase: string, private store: EvidenceStore, page: Page) {
    const onPagehide = (event: { isTrusted: boolean }) => {
      if (!event.isTrusted) return;
      page.removeEventListener('pagehide', onPagehide);
      this.hidden = true;
      if (this.transportFailed) this.store.setItem(CANCELLATION_KEY, `${this.phase}:pagehide`);
    };
    page.addEventListener('pagehide', onPagehide);
  }

  navigate(action: () => void): void {
    this.navigating = true;
    action();
  }

  recordTransport(): boolean {
    if (!this.navigating) return false;
    this.transportFailed = true;
    this.store.setItem(CANCELLATION_KEY, `${this.phase}:${this.hidden ? 'pagehide' : 'pending'}`);
    return true;
  }
}

export function confirmHistoryNavigation(store: EvidenceStore, phase: string, terminalCode?: string): void {
  const evidence = store.getItem(CANCELLATION_KEY);
  if (evidence === null && (!terminalCode || terminalCode === 'navigated' || terminalCode === 'closed')) return;
  if (evidence !== `${phase}:pagehide` || (terminalCode && terminalCode !== 'transport')) {
    throw new Error(`history cancellation not verified phase=${phase} terminal=${terminalCode} evidence=${evidence}`);
  }
  // Do not rewrite a transport terminal error: the next document has already
  // checked native cancellation and fresh admission before consuming this proof.
  store.removeItem(CANCELLATION_KEY);
}
