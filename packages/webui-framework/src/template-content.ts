// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { TemplateBlockMeta } from './template-types.js';

/** Immutable attributes needed to prepare one external component stylesheet. */
export interface TemplateStylesheetDescriptor {
  readonly crossOrigin: string | null;
  readonly disabled: boolean;
  readonly elementIndex: number;
  readonly href: string;
  readonly hasInlineStyles: boolean;
  readonly hasUnsupportedAttributes: boolean;
  readonly integrity: string;
  readonly media: string;
  readonly referrerPolicy: string;
  readonly title: string;
  readonly type: string;
}

interface TemplateContent {
  readonly fragment: DocumentFragment;
  readonly stylesheets: readonly TemplateStylesheetDescriptor[];
}

const EMPTY_STYLESHEETS: readonly TemplateStylesheetDescriptor[] = Object.freeze([]);
const templateContentCache = new WeakMap<TemplateBlockMeta, TemplateContent>();

/** Clone cached template DOM for one client-created block instance. */
export function cloneTemplateContent(meta: TemplateBlockMeta): DocumentFragment {
  return getTemplateContent(meta).fragment.cloneNode(true) as DocumentFragment;
}

/** Return external stylesheet descriptors discovered during the template's single parse. */
export function getTemplateStylesheets(
  meta: TemplateBlockMeta,
): readonly TemplateStylesheetDescriptor[] {
  return getTemplateContent(meta).stylesheets;
}

function getTemplateContent(meta: TemplateBlockMeta): TemplateContent {
  let cached = templateContentCache.get(meta);
  if (cached) return cached;

  const template = document.createElement('template');
  template.innerHTML = meta.h;
  const fragment = template.content;
  cached = {
    fragment,
    stylesheets: collectStylesheetDescriptors(fragment),
  };
  templateContentCache.set(meta, cached);
  return cached;
}

function collectStylesheetDescriptors(
  fragment: DocumentFragment,
): readonly TemplateStylesheetDescriptor[] {
  const elements = fragment.querySelectorAll('*');
  const hasInlineStyles = fragment.querySelector('style') !== null;
  let stylesheets: TemplateStylesheetDescriptor[] | undefined;
  for (let i = 0; i < elements.length; i++) {
    const element = elements[i];
    if (element.localName !== 'link') continue;
    const link = element as HTMLLinkElement;
    if (
      !link.relList.contains('stylesheet') ||
      link.relList.contains('alternate')
    ) {
      continue;
    }
    const href = link.getAttribute('href');
    if (!href) continue;
    (stylesheets ??= []).push({
      crossOrigin: link.getAttribute('crossorigin'),
      disabled: link.disabled,
      elementIndex: i + 1,
      href,
      hasInlineStyles,
      hasUnsupportedAttributes: containsUnsupportedAttributes(link),
      integrity: link.integrity,
      media: link.media,
      referrerPolicy: link.referrerPolicy,
      title: link.title,
      type: link.type,
    });
  }
  return stylesheets ?? EMPTY_STYLESHEETS;
}

function containsUnsupportedAttributes(link: HTMLLinkElement): boolean {
  const attributes = link.attributes;
  for (let i = 0; i < attributes.length; i++) {
    switch (attributes[i].name) {
      case 'crossorigin':
      case 'disabled':
      case 'href':
      case 'integrity':
      case 'media':
      case 'referrerpolicy':
      case 'rel':
      case 'title':
      case 'type':
        break;
      default:
        return true;
    }
  }
  return false;
}
