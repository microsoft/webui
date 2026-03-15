# WebUI Store — WebUI Marketplace Demo

A blazing-fast, server-rendered marketplace dashboard built with **WebUI** and **actix-web**.

This demo proves WebUI's SSR performance: 1,000+ products rendered server-side in sub-millisecond time, with seamless FAST-HTML client hydration for interactivity.

## Architecture

- **Server**: actix-web (Rust) with 3 SSR page routes + cart API
- **Templates**: WebUI binary protocol — compiled once, rendered per-request
- **Client**: FAST-HTML hydration for cart, search, gallery, variant selection
- **Design**: Atomic Design (atoms → molecules → organisms → pages)
- **Theme**: WebUI dark theme inspired by copilot.microsoft.com

## Quick Start

```bash
# Install client dependencies
pnpm install

# Run the server.
pnpm start

# Open http://localhost:3100
```

## Pages

| Page | URL | Description |
|------|-----|-------------|
| Homepage | `/` | 3-item hero grid + product carousel |
| Search | `/search?q=&sort=` | Category sidebar + product grid + sort |
| Category | `/search/{category}` | Filtered by category |
| Product | `/product/{handle}` | Gallery + variants + add-to-cart |

## Performance

Templates are compiled to binary protocol at server startup. Per-request rendering only injects state data — no template parsing, no JavaScript execution on the server.
