// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// WebUI Docs — hydration entry point.
// Interactive components are discovered from the active template regions.

// Hash anchor scrolling
if (window.location.hash) {
  const rawAnchor = window.location.hash.slice(1);
  let anchor = rawAnchor;
  try {
    anchor = decodeURIComponent(rawAnchor);
  } catch (error) {
    if (!(error instanceof URIError)) {
      throw error;
    }
  }
  document.getElementById(anchor)?.scrollIntoView();
}
