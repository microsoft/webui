---
name: docs-sync
description: Keep user-facing docs and DESIGN specification aligned with behavior and API changes.
---

# Docs Synchronization Workflow

Use this skill whenever a change touches user-visible behavior, APIs, or contracts.

## When to update docs

| What changed | Update |
|-------------|--------|
| CLI flags or commands | `docs/guide/cli/index.md` + `docs/guide/ai.md` (CLI section) |
| Template syntax or directives | `docs/guide/concepts/directives/` + `docs/guide/ai.md` |
| Component authoring model | `docs/guide/concepts/interactivity.md` + `docs/guide/ai.md` |
| Hydration markers or mechanism | `docs/guide/concepts/hydration.md` + `DESIGN.md` (WebUI Framework Plugin) |
| Routing behavior | `docs/guide/concepts/routing.md` + `docs/guide/ai.md` |
| State management or path resolution | `docs/guide/concepts/state-management/index.md` |
| Handler API (Rust, Node, FFI) | `docs/guide/concepts/handlers/` + `docs/guide/integrations.md` |
| Protocol fields or fragment types | `DESIGN.md` (Protocol Specification) |
| Plugin system (parser or handler) | `docs/guide/concepts/plugins/index.md` + `DESIGN.md` |
| Performance characteristics | `docs/guide/concepts/performance.md` |
| Public API (Rust crate, npm package) | `DESIGN.md` + relevant handler/integration docs |
| Error variants or error messages | `DESIGN.md` |
| `@microsoft/webui-framework` decorators or API | `docs/guide/concepts/interactivity.md` + `docs/guide/ai.md` + `packages/webui-framework/README.md` |
| `@microsoft/webui-router` behavior | `docs/guide/concepts/routing.md` + `packages/webui-router/README.md` |

## DESIGN.md rules

`DESIGN.md` is the living technical specification. Update it in the same commit when modifying:

- Public APIs or type signatures
- Protocol fields or fragment types
- Behavioral contracts (matching semantics, expression evaluation, state resolution)
- Error variants
- SSR marker formats
- Metadata object format

If `DESIGN.md` and the code disagree, that is a bug - fix both.

## docs/ rules

Update `docs/` in the same commit when the change is user-visible:

- CLI usage or flags changed
- Template syntax or rendering output changed
- Integration behavior that external developers depend on
- New features or removed features

Keep protocol internals out of general user docs. The `docs/guide/ai.md` file is the single-page AI reference and should be kept in sync with all other docs.

## Validation

```bash
cd docs && pnpm build
```

This catches broken links, VitePress syntax errors (unescaped `{{` outside code blocks), and missing pages. Run it when docs are changed.

## Style rules for docs

- No emdashes (` - `). Use hyphens (` - `).
- Escape `{{` outside fenced code blocks with `<code v-pre>{{expr}}</code>`.
- Use correct CLI flag names (check `crates/webui-cli/src/commands/`).
- Verify SSR markers match source code (`crates/webui-handler/src/plugin/webui.rs`).
