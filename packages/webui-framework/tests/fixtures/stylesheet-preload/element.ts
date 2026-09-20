// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  registerTemplateData,
  WebUIElement,
} from '../../../src/index.js';
import {
  preloadComponentAssetStyles,
} from '../../../src/element/link-styles.js';

const TAG_NAME = 'test-preloaded-stylesheet';
const STYLESHEET_HREF = '/stylesheet-preload/early.css';
const ATTRIBUTE_TAG_NAME = 'test-attribute-stylesheet';
const ATTRIBUTE_STYLESHEET_HREF = '/stylesheet-preload/attributes.css';

preloadComponentAssetStyles([
  STYLESHEET_HREF,
  new URL(STYLESHEET_HREF, document.baseURI).href,
]);
preloadComponentAssetStyles([STYLESHEET_HREF]);

class TestPreloadedStylesheet extends WebUIElement {}
class TestAttributeStylesheet extends WebUIElement {}

declare global {
  interface Window {
    __preloadAttributeStylesheetFixture?: () => void;
    __preloadUnsupportedStylesheetFixture?: () => void;
    __registerAttributeStylesheetFixture?: () => void;
    __registerPreloadedStylesheetFixture?: () => void;
  }
}

window.__registerPreloadedStylesheetFixture = (): void => {
  registerTemplateData({
    [TAG_NAME]: {
      h:
        `<link rel="stylesheet" href="${STYLESHEET_HREF}">` +
        '<p class="early-label">Preloaded stylesheet</p>',
      sd: 1,
    },
  });
  TestPreloadedStylesheet.define('test-preloaded-stylesheet');
  document.body.appendChild(document.createElement(TAG_NAME));
};

window.__preloadAttributeStylesheetFixture = (): void => {
  preloadComponentAssetStyles([ATTRIBUTE_STYLESHEET_HREF]);
};

window.__preloadUnsupportedStylesheetFixture = (): void => {
  preloadComponentAssetStyles(['/stylesheet-preload/unsupported.css']);
};

window.__registerAttributeStylesheetFixture = (): void => {
  registerTemplateData({
    [ATTRIBUTE_TAG_NAME]: {
      h:
        `<link rel="stylesheet" href="${ATTRIBUTE_STYLESHEET_HREF}" ` +
        'crossorigin="anonymous" referrerpolicy="no-referrer">' +
        '<p class="attribute-label">Attribute stylesheet</p>',
      sd: 1,
    },
  });
  TestAttributeStylesheet.define('test-attribute-stylesheet');
  document.body.appendChild(document.createElement(ATTRIBUTE_TAG_NAME));
};
