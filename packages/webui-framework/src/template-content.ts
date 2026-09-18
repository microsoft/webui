// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { TemplateBlockMeta } from './template-types.js';
import { collectTemplateElements } from './element/markers.js';

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
  outlets?: readonly number[];
  rootOutlet?: boolean;
}

const EMPTY_STYLESHEETS: readonly TemplateStylesheetDescriptor[] = Object.freeze([]);
const NO_LINK_STYLESHEETS: readonly TemplateStylesheetDescriptor[] = Object.freeze([]);
const templateContentCache = new WeakMap<TemplateBlockMeta, TemplateContent>();

/** Clone cached template DOM for one client-created block instance. */
export function cloneTemplateContent(meta: TemplateBlockMeta): DocumentFragment {
  return getTemplateFragment(meta).cloneNode(true) as DocumentFragment;
}

/** Return cached, context-preserving template DOM for SSR path mapping. */
export function getTemplateFragment(meta: TemplateBlockMeta): DocumentFragment {
  return getTemplateContent(meta).fragment;
}

/** Cached compiler element indices for client-created outlet ranges. */
export function getTemplateOutlets(meta: TemplateBlockMeta): readonly number[] | undefined {
  return getTemplateContent(meta).outlets;
}

/** Whether a block needs range ownership for a top-level outlet. */
export function templateHasRootOutlet(meta: TemplateBlockMeta): boolean {
  const cached = templateContentCache.get(meta);
  if (cached) return cached.rootOutlet === true;
  return templateHtmlMayContainTag(meta.h, 'outlet')
    && getTemplateContent(meta, undefined, true).rootOutlet === true;
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
  return templateHtmlMayContainTag(html, 'link');
}

function templateHtmlMayContainTag(html: string, tag: string): boolean {
  for (let i = html.indexOf('<'); i !== -1; i = html.indexOf('<', i + 1)) {
    let length = 0;
    while (length < tag.length && asciiLower(html.charCodeAt(i + length + 1)) === tag.charCodeAt(length)) length++;
    if (length !== tag.length) continue;
    const end = i + tag.length + 1;
    const next = html.charCodeAt(end);
    if (
      end === html.length ||
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
  mayContainOutlet?: boolean,
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
  if (mayContainOutlet ?? templateHtmlMayContainTag(meta.h, 'outlet')) {
    const elements = collectTemplateElements(fragment);
    let outlets: number[] | undefined;
    for (let i = 1; i < elements.length; i++) {
      const element = elements[i] as Element;
      if (element.localName !== 'outlet') continue;
      (outlets ??= []).push(i);
      if (element.parentNode === fragment) cached.rootOutlet = true;
    }
    if (outlets) cached.outlets = outlets;
  }
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
