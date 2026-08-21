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
const NO_LINK_STYLESHEETS: readonly TemplateStylesheetDescriptor[] = Object.freeze([]);
const templateContentCache = new WeakMap<TemplateBlockMeta, TemplateContent>();

/** Clone cached template DOM for one client-created block instance. */
export function cloneTemplateContent(meta: TemplateBlockMeta): DocumentFragment {
  return getTemplateContent(meta).fragment.cloneNode(true) as DocumentFragment;
}

/** Return external stylesheet descriptors discovered during the template's single parse. */
export function getTemplateStylesheets(
  meta: TemplateBlockMeta,
): readonly TemplateStylesheetDescriptor[] | undefined {
  const stylesheets = getTemplateContent(meta, true).stylesheets;
  return stylesheets === NO_LINK_STYLESHEETS ? undefined : stylesheets;
}

/** Return whether template HTML may contain a `<link>` start tag. */
export function templateHtmlMayContainLink(html: string): boolean {
  for (let i = 0; i <= html.length - 5; i++) {
    if (html.charCodeAt(i) !== 60) continue;
    if (
      asciiLower(html.charCodeAt(i + 1)) !== 108 ||
      asciiLower(html.charCodeAt(i + 2)) !== 105 ||
      asciiLower(html.charCodeAt(i + 3)) !== 110 ||
      asciiLower(html.charCodeAt(i + 4)) !== 107
    ) {
      continue;
    }
    const next = html.charCodeAt(i + 5);
    if (
      i + 5 === html.length ||
      next === 9 ||
      next === 10 ||
      next === 12 ||
      next === 13 ||
      next === 32 ||
      next === 47 ||
      next === 62
    ) {
      return true;
    }
  }
  return false;
}

function getTemplateContent(
  meta: TemplateBlockMeta,
  mayContainLink?: boolean,
): TemplateContent {
  let cached = templateContentCache.get(meta);
  if (cached) return cached;

  const template = document.createElement('template');
  template.innerHTML = meta.h;
  const fragment = template.content;
  const stylesheets = (mayContainLink ?? templateHtmlMayContainLink(meta.h))
    ? collectStylesheetDescriptors(fragment)
    : NO_LINK_STYLESHEETS;
  cached = { fragment, stylesheets };
  templateContentCache.set(meta, cached);
  return cached;
}

function collectStylesheetDescriptors(
  fragment: DocumentFragment,
): readonly TemplateStylesheetDescriptor[] {
  const elements = fragment.querySelectorAll('*');
  let hasLink = false;
  let hasInlineStyles: boolean | undefined;
  let stylesheets: TemplateStylesheetDescriptor[] | undefined;
  for (let i = 0; i < elements.length; i++) {
    const element = elements[i];
    if (element.localName !== 'link') continue;
    hasLink = true;
    const link = element as HTMLLinkElement;
    if (
      !link.relList.contains('stylesheet') ||
      link.relList.contains('alternate')
    ) {
      continue;
    }
    const href = link.getAttribute('href');
    if (!href) continue;
    hasInlineStyles ??= fragment.querySelector('style') !== null;
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
  return stylesheets ?? (hasLink ? EMPTY_STYLESHEETS : NO_LINK_STYLESHEETS);
}

function asciiLower(code: number): number {
  return code >= 65 && code <= 90 ? code + 32 : code;
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
