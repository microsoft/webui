// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import {
  rewriteCssUrls,
  templateMayContainLinkStyles,
} from './link-styles.js';

describe('Link stylesheet scanning', () => {
  test('skips Link-mode mount work for templates without link elements', () => {
    assert.equal(
      templateMayContainLinkStyles({ h: '<p class="card">Ready</p>' }),
      false,
    );
    assert.equal(
      templateMayContainLinkStyles({ h: '<link-card></link-card>' }),
      false,
    );
  });

  test('resolves quoted and unquoted URLs against the stylesheet', () => {
    assert.equal(
      rewriteCssUrls(
        '.a{background:url(../icons/a.svg)}.b{mask:url("./b.svg#mask")}',
        'https://cdn.example.test/components/card/card.css',
      ),
      '.a{background:url("https://cdn.example.test/components/icons/a.svg")}' +
      '.b{mask:url("https://cdn.example.test/components/card/b.svg#mask")}',
    );
    assert.equal(
      rewriteCssUrls(
        ".a{background:url('icon.svg')}",
        "https://cdn.example.test/components/card's/card.css",
      ),
      ".a{background:url('https://cdn.example.test/components/card\\'s/icon.svg')}",
    );
  });

  test('preserves fragment URLs and ignores URL text in strings or comments', () => {
    const css = '.a{filter:url(#blur);content:"url(fake.svg)"}/* url(old.svg) */';
    assert.equal(
      rewriteCssUrls(css, 'https://cdn.example.test/components/card.css'),
      css,
    );
  });

  test('falls back for CSS URL forms that cannot be safely rewritten', () => {
    assert.equal(
      rewriteCssUrls(
        '.a{background:image-set("small.png" 1x,"large.png" 2x)}',
        'https://cdn.example.test/components/card.css',
      ),
      undefined,
    );
    assert.equal(
      rewriteCssUrls(
        '.a{background:url(var(--image))}',
        'https://cdn.example.test/components/card.css',
      ),
      undefined,
    );
    assert.equal(
      rewriteCssUrls(
        '.a{background:u\\72l(escaped.png)}',
        'https://cdn.example.test/components/card.css',
      ),
      undefined,
    );
  });
});
