// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// WebUI Docs — hydration entry point.
// Imports interactive components for client-side behavior.

import "./docs-theme-toggle/docs-theme-toggle.js";
import "./docs-search/docs-search.js";

// Load site-specific component bundle (if exists)
const base =
  document.querySelector("meta[name='base']")?.getAttribute("content") || "/";
const s = document.createElement("script");
s.type = "module";
s.src = base + "components.js";
document.head.appendChild(s);

// Hash anchor scrolling
if (window.location.hash) {
  const el = document.querySelector(window.location.hash);
  if (el) el.scrollIntoView();
}

// Mobile sidebar toggle
const mobileBtn = document.getElementById("mobile-menu-btn");
if (mobileBtn) {
  mobileBtn.addEventListener("click", () => {
    const sidebar = document.querySelector(".sidebar");
    if (sidebar) sidebar.classList.toggle("open");
  });
}
