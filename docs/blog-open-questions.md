# Open Questions: WebUI Blog Series

These are unresolved questions and investigations needed before publishing the blog series. Blogs 1–3 are ready for editorial review. Blog 4 needs team input on several items below.

---

## 1. In-Product Examples (Review before publishing Blog 4)

**Status:** Blog 4 currently references Downloads, History, Settings, Wallet, and Permission Prompts generically. We need confirmation on what we can name publicly.

**Action items:**
- [ ] Get approval from feature teams to name specific Edge features in the blog
- [ ] Confirm the "12+ BTR features" count is accurate and current
- [ ] Get before/after performance numbers for at least 2–3 migrated features (LCP, bundle size)
- [ ] Decide if we can show actual Edge UI screenshots or keep it abstract
- [ ] Verify none of the named features have been removed or renamed since drafting

---

## 2. React vs BTR Code Comparison (Review before publishing Blog 4)

**Status:** Blog 4 includes a simplified React vs BTR before/after comparison for a download item component. This is illustrative but synthetic.

**Action items:**
- [ ] Review the code comparison for accuracy — does it fairly represent both approaches?
- [ ] Find a real (sanitized) component that was migrated, if the synthetic example isn't compelling enough
- [ ] Quantify bundle size reduction with actual numbers (React bundle vs BTR output for the same page)
- [ ] Document developer experience differences (build steps, debugging, testing) — potential follow-up content

---

## 3. Native Performance Comparison (Potential Blog 5 or appendix)

**Problem:** WebUI's SSR performance is competitive enough that comparing with native UI frameworks is worth investigating. Should we make that claim with data?

**Action items:**
- [ ] Build a WinUI 3 equivalent of the SSR showdown benchmark (Windows native)
- [ ] Build a SwiftUI/AppKit equivalent (macOS native)
- [ ] Compare: WebUI SSR render time vs native UI framework render-to-screen time
- [ ] Decide if this comparison is fair (SSR produces HTML strings that still need browser parsing; native renders directly to pixels)
- [ ] Decide where this belongs: Blog 4 appendix? Separate blog post? Repo documentation?

**Risk:** Apples-to-oranges comparison. SSR produces HTML that still needs browser parsing. Native renders directly. May need careful framing — e.g., "time to interactive" as the common metric rather than raw render time.

---

## 4. Device Class Performance (Strengthens Blog 1 and Blog 4)

**Problem:** The original blog's value prop was "fast on low-end devices." Blog 4 claims ~260ms LCP on BTR vs ~1–2s on React. We should back this up with controlled data across machine classes.

**Action items:**
- [ ] Run WebUI SSR showdown on 3 device classes:
  - Low-end: 4GB RAM, no SSD (or equivalent cloud VM)
  - Mid-range: 8GB RAM, SSD
  - High-end: 16GB RAM, SSD
- [ ] Measure P95 latency for each, not just averages
- [ ] Compare how WebUI degrades vs React/Fastify on constrained hardware
- [ ] If Edge telemetry is available, show real-world P50/P95 across device populations

**Hypothesis:** WebUI's advantage should be *larger* on low-end devices because it avoids GC pauses, JIT warmup, and memory pressure from the JS runtime. If confirmed, this is a powerful data point for Blog 1's "hits hardest on the devices that can least afford it" argument.

---

## 5. Editorial Review Checklist

Before publishing any blog:
- [ ] Legal review of open-source claims and MIT license references
- [ ] Verify all benchmark numbers match current codebase (run `cargo xtask bench all`)
- [ ] Confirm GitHub repo URL is live and public (https://github.com/microsoft/webui)
- [ ] Confirm docs site URL is live (https://microsoft.github.io/webui)
- [ ] Confirm playground URL is live (https://microsoft.github.io/webui/playground/)
- [ ] Review code samples for correctness against current API surface
- [ ] Get sign-off from Edge leadership on Blog 4's internal details

---

## Decision Log

| Date | Decision | Rationale |
|------|----------|-----------|
| 2026-03-28 | Remove Browser Essentials from all blogs | Feature was removed from Edge |
| 2026-03-28 | Split into 4-blog series | Why → How it works → How to build → How Edge uses it |
| 2026-03-28 | Defer native perf comparison | Needs investigation; may not be fair comparison |
| 2026-03-28 | Add "web platform bet" to Blog 1 | Strategic argument for portability (browser, PWA, WebView2) |
| 2026-03-28 | Blog 3: platform maturity + opinionated design | React's flexibility enables bloat; WebUI structurally prevents it |
| 2026-03-28 | Blog 4: spectrum of complexity | Show BTR scales from lightweight to complex interactive |

---

## Blog Series Summary

| # | File | Status |
|---|------|--------|
| 1 | `blog-why-we-rebuilt-web-rendering.md` | ✅ Ready for editorial review |
| 2 | `blog-inside-webui-technical-deep-dive.md` | ✅ Ready for editorial review |
| 3 | `blog-building-interactive-apps.md` | ✅ Ready for editorial review |
| 4 | `blog-from-react-to-btr.md` | ⚠️ Needs team input on sections 1, 2 above |
