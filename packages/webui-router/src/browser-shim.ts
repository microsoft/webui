// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Minimal browser API stubs so webui-router modules can load in Node.
 * Must be imported before any router code.
 */

interface BrowserGlobals {
  HTMLElement: unknown;
  customElements: {
    get(name: string): unknown;
    define(name: string, ctor: unknown): void;
    whenDefined(name: string): Promise<unknown>;
  };
  document: {
    createElement(): Record<string, unknown>;
    getElementById(id: string): null;
    querySelector(): null;
    querySelectorAll(): unknown[];
    addEventListener(): void;
    removeEventListener(): void;
    body: { children: never[]; appendChild(): void };
    createDocumentFragment(): { appendChild(): void };
    startViewTransition: undefined;
    head: { appendChild(): void };
  };
  window: typeof globalThis;
  dispatchEvent(event: Event): boolean;
  navigation: {
    addEventListener(): void;
    removeEventListener(): void;
  };
  location: {
    href: string;
    origin: string;
    pathname: string;
  };
}

const g = globalThis as unknown as Partial<BrowserGlobals>;

if (typeof HTMLElement === 'undefined') {
  g.HTMLElement = class HTMLElement {};
}
if (typeof customElements === 'undefined') {
  const registry = new Map<string, unknown>();
  g.customElements = {
    get: (name: string) => registry.get(name),
    define: (name: string, ctor: unknown) => { registry.set(name, ctor); },
    whenDefined: (_name: string) => Promise.resolve(),
  };
}
if (typeof document === 'undefined') {
  g.document = {
    createElement: () => ({ setAttribute() {}, style: {} }),
    getElementById: () => null,
    querySelector: () => null,
    querySelectorAll: () => [],
    addEventListener() {},
    removeEventListener() {},
    body: { children: [], appendChild() {} },
    createDocumentFragment: () => ({ appendChild() {} }),
    startViewTransition: undefined,
    head: { appendChild() {} },
  };
}
if (typeof window === 'undefined') {
  g.window = globalThis;
}
if (typeof dispatchEvent === 'undefined') {
  g.dispatchEvent = () => true;
}
if (!g.navigation) {
  g.navigation = {
    addEventListener() {},
    removeEventListener() {},
  };
}
if (typeof location === 'undefined') {
  g.location = {
    href: 'http://localhost/',
    origin: 'http://localhost',
    pathname: '/',
  };
}
