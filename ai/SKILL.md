---
name: webui-reference
description: Use when writing or reviewing WebUI application or framework code. Load the application's installed reference or the WebUI source checkout's canonical reference.
---

Before writing or reviewing WebUI code, select the reference for the project
being changed. Resolve paths from that project, never from this skill's
installation directory, even when the skill is installed globally.

- **WebUI framework source checkout (`microsoft/webui`), including its workspace
  examples:** read and follow `docs/ai.md` from the framework repository root.
  Do not require an installed or generated package copy.
- **Consuming application:** read and follow `@microsoft/webui/ai.md` from that
  application's installed dependencies. Resolve the package from the application's
  directory so app-local and hoisted dependencies both work; do not assume the
  workspace root's `node_modules` contains the application's version.

Reread the selected reference after upgrades or checkout changes. If unavailable,
report the missing reference instead of using cached, latest-version, or
implementation-derived guidance. The source-checkout rule is not a fallback for
applications with missing dependencies or older releases without `ai.md`.
