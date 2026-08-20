// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// WebUI Docs — hydration entry point.
// Imports interactive components for client-side behavior.

import "./docs-site-navigation/docs-site-navigation.js";

// Hash anchor scrolling
if (window.location.hash) {
  const anchor = decodeURIComponent(window.location.hash.slice(1));
  document.getElementById(anchor)?.scrollIntoView();
}
