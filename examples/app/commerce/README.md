<!-- Copyright (c) Microsoft Corporation. -->
<!-- Licensed under the MIT license. -->

# Acme Commerce Store

A full-featured commerce demo built with WebUI — server-side rendered with client-side navigation, nested routes, and view transitions.

## What It Demonstrates

- **Nested routing** with `<route>` and `<outlet />`
- **Client-side navigation** via `@microsoft/webui-router`
- **SSR + hydration** with FAST-HTML
- **View transitions** for smooth page changes
- **Category filtering** and **sort options**
- **Product gallery** with thumbnail navigation
- **Shopping cart** with add/remove/quantity
- **Mobile responsive** layout with CSS-only breakpoints
- **Visual regression tests** with Playwright

## Quick Start

```bash
# From the repository root
cd examples/app/commerce

# Install dependencies
pnpm install

# Start the dev server (builds + serves on port 3004)
pnpm start
```

Then open http://127.0.0.1:3004

## Running Tests

```bash
# Start the server first
pnpm start:server

# In another terminal, run Playwright tests
pnpm test

# Update visual regression snapshots
pnpm test:update-snapshots
```

## Project Structure

```
commerce/
├── src/                    # Frontend source
│   ├── index.html          # Route declarations + global styles
│   ├── index.ts            # Hydration + router setup
│   ├── atoms/              # Small reusable elements (icon, price, image)
│   ├── molecules/          # Composite elements (product-label, search-bar)
│   ├── organisms/          # Complex components (navbar, cart, gallery)
│   └── pages/              # Route page components
├── server/                 # Custom Rust server (marketplace-api)
│   └── src/
│       ├── app.rs          # Actix-web app setup
│       ├── server.rs       # Route handlers (SSR + JSON partials)
│       ├── frontend.rs     # WebUI protocol rendering
│       └── state/          # State resolution per route
├── tests/                  # Playwright E2E tests
│   └── commerce.spec.ts    # 42 tests (desktop + mobile)
├── dist/                   # Built client bundle
└── playwright.config.ts    # Test config (chromium + mobile)
```

## Route Structure

```html
<route path="/" component="mp-app">
  <route path="" component="mp-page-home" exact />
  <route path="search" component="mp-page-search">
    <route path="" component="mp-product-grid" exact />
    <route path=":category" component="mp-product-grid" exact />
  </route>
  <route path="product/:handle" component="mp-page-product" exact />
  <route path="about" component="mp-page-about" exact />
  <route path="terms-conditions" component="mp-page-terms" exact />
  <route path="shipping-return-policy" component="mp-page-shipping" exact />
  <route path="privacy-policy" component="mp-page-privacy" exact />
  <route path="frequently-asked-questions" component="mp-page-faq" exact />
</route>
```
