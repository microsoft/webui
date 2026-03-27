# `@microsoft/webui-test-support`

Private workspace-only helpers for WebUI package tests.

This package exists so `webui-framework` and `webui-router` can share the same
test infrastructure instead of each package carrying slightly different copies
of it.

## What lives here

- The compiled-template fixture DSL used by browser-side test fixtures.
- Shared fixture bundling helpers for turning `tests/fixtures/**` TypeScript
  entries into runnable browser bundles.
- Shared fixture server helpers for Playwright-backed fixture servers.

## Why it exists

The framework and router tests both need to:

- register compiled template metadata consistently,
- bundle browser fixture apps,
- serve built fixture assets to Playwright,
- keep test-only protocol metadata helpers aligned with the runtime contract.

Centralizing that logic here reduces drift and makes test infrastructure changes
land in one place.

## Publishing

This package is intentionally **not published**.

- Package name: `@microsoft/webui-test-support`
- `private: true`

It is for workspace reuse only.

## Exports

- `@microsoft/webui-test-support` — template metadata helpers safe for browser
  fixture bundles.
- `@microsoft/webui-test-support/fixture-build` — Node-only helpers for building
  fixture bundles.
- `@microsoft/webui-test-support/fixture-server` — Node-only helpers for
  Playwright fixture servers.
