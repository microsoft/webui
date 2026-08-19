// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

self.addEventListener('fetch', event => {
  const url = new URL(event.request.url);
  if (
    url.pathname.endsWith('/child.css') ||
    url.pathname.endsWith('/styles/relative.css')
  ) {
    event.respondWith(fetch(event.request));
  }
});
