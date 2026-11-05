---
name: webui-reference
description: Use when writing or reviewing WebUI application or framework code. Load the application's installed reference or the WebUI source checkout's canonical reference.
---

Before writing or reviewing WebUI code, select the reference for the project
being changed. Resolve paths from that project, never from this skill's
installation directory, even when the skill is installed globally. Read the
core reference first, then the topic guides relevant to the task. Do not load
every guide by default.

- **WebUI framework source checkout (`microsoft/webui`), including its workspace
  examples:** read `docs/ai.md` at the repository root and the relevant pages
  under that checkout's `docs/`. In source Markdown, a site-root link such as
  `/learn/lazy` maps to `docs/learn/lazy.md`; resolve other relative links from
  their containing page. Add `.md` or use the directory's `index.md`, preserving
  any section fragment. Do not require an installed or generated package copy.
- **Consuming application:** from the application's directory, locate its
  installed reference with `node -p "require.resolve('@microsoft/webui/ai.md')"`.
  Read that file and follow its relative Markdown links to the `docs/` directory
  beside it. These files ship with `@microsoft/webui`, not
  `@microsoft/webui-framework`, and work offline. Node package resolution
  handles app-local and hoisted installations; do not assume a workspace-root
  `node_modules` contains the application's version.

The package's documentation is built from the same release as its CLI. Keep
WebUI packages at matching versions; if the installed framework and CLI versions
disagree, report the mismatch instead of assuming either reference applies.
When a guide contains `<webui-press-tabs>`, its panels correspond to the tab
labels in order. Interactive playgrounds and benchmark data are site-only;
do not infer their contents from placeholder markup.

Reread the selected reference after upgrades or checkout changes. If unavailable,
report the missing reference or topic guide and, when available, the installed
package version instead of using cached, latest-version, or
implementation-derived guidance.
The source-checkout rule is not a fallback for applications with missing
dependencies or older releases without the bundled guides.
