// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

/**
 * A single feed island. `feed-item` instances are the unit of streaming
 * hydration for the feed — the feed's `<section>` container itself is
 * never hydrated. Each `feed-item` is its own boundary root, so the
 * container it lives in is never treated as a hydratable component that
 * a later boundary could append into. Each item's state (`post-id`,
 * `author`, `text`, `like-count`) is carried entirely in its own
 * attributes, projected once from its own batch's boundary checkpoint, so
 * one chunk's items can never read or consume another chunk's state.
 *
 * `likeCount` follows the numeric-attribute-as-string convention used
 * elsewhere in this codebase (see `mp-cart-panel`'s `subtotal`/`taxes`):
 * the server projects it as a string, and client mutations round-trip
 * through `Number`/`String` rather than switching representations.
 */
export class FeedItem extends WebUIElement {
  @attr({ attribute: 'post-id' }) postId = '';
  @attr author = '';
  @attr text = '';
  @attr({ attribute: 'like-count' }) likeCount = '0';

  onLikeClick(): void {
    this.likeCount = String(Number(this.likeCount) + 1);
  }
}

FeedItem.define('feed-item');
