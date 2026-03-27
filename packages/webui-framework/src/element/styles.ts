// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * CSS module stylesheet cache.
 *
 * When the Rust compiler uses the "module" CSS strategy, component styles
 * are delivered as `<style type="module" specifier="tag-name">` elements
 * in the HTML payload.  This module parses those definitions into
 * constructable `CSSStyleSheet` objects and caches them so that each
 * component instance can adopt the stylesheet into its shadow root via
 * `adoptedStyleSheets` without re-parsing the CSS.
 */

const moduleStylesheets = new Map<string, CSSStyleSheet>();

function findModuleStyleDefinition(specifier: string): HTMLStyleElement | null {
  const definitions = document.querySelectorAll('style[type="module"][specifier]');
  for (let index = 0; index < definitions.length; index += 1) {
    const definition = definitions[index];
    if (definition.getAttribute('specifier') === specifier) {
      return definition as HTMLStyleElement;
    }
  }

  return null;
}

export function getModuleStylesheet(specifier: string): CSSStyleSheet {
  const cached = moduleStylesheets.get(specifier);
  if (cached) {
    return cached;
  }

  if (typeof CSSStyleSheet === 'undefined' || typeof CSSStyleSheet.prototype.replaceSync !== 'function') {
    throw new Error(`Module CSS for "${specifier}" requires constructable stylesheets support.`);
  }

  const definition = findModuleStyleDefinition(specifier);
  if (!definition) {
    throw new Error(`Missing CSS module definition for "${specifier}".`);
  }

  const stylesheet = new CSSStyleSheet();
  stylesheet.replaceSync(definition.textContent ?? '');
  moduleStylesheets.set(specifier, stylesheet);
  return stylesheet;
}
