---
name: docs-sync
description: Keep user-facing docs current and DESIGN aligned with architectural changes.
---

# Docs Synchronization Workflow

Use this skill whenever a change touches user-visible behavior, APIs, or contracts.

## When to update docs

| What changed | Update |
|-------------|--------|
| CLI flags or commands | `docs/guide/cli/index.md` + `docs/ai.md` (Build and run section) |
| Template syntax or directives | `docs/guide/concepts/directives/` + `docs/ai.md` |
| Component authoring model | `docs/guide/concepts/interactivity.md` + `docs/ai.md` |
| Hydration markers or mechanism | `specs/hydration.md` + producer/consumer tests; update `docs/guide/concepts/hydration.md` if user-visible, `DESIGN.md` only if hydration architecture changes |
| Routing behavior | `docs/guide/concepts/routing.md` + `docs/ai.md` |
| State management or path resolution | `docs/guide/concepts/state-management/index.md` |
| Handler API (Rust, Node, FFI) | `docs/guide/concepts/handlers/` + `docs/guide/integrations.md` |
| Protocol fields or fragment types | `crates/webui-protocol/proto/webui.proto` + consumers/tests; `DESIGN.md` only if the graph or ownership model changes |
| Internal subsystem or cross-language contract | Relevant `specs/` reference + producer/consumer tests; `DESIGN.md` only if the architecture changes |
| Plugin system (parser or handler) | `docs/guide/concepts/plugins/index.md`; `DESIGN.md` if plugin responsibilities change |
| Performance characteristics | `docs/guide/concepts/performance.md` |
| Public API (Rust crate, npm package) | Relevant handler/integration docs; `DESIGN.md` only if an architectural boundary changes |
| Error variants or error messages | Defining source/tests and relevant public troubleshooting docs; `DESIGN.md` only if error-handling architecture changes |
| `@microsoft/webui-framework` decorators or API | `docs/guide/concepts/interactivity.md` + `docs/ai.md` + `packages/webui-framework/README.md` |
| `@microsoft/webui-router` behavior | `docs/guide/concepts/routing.md` + `packages/webui-router/README.md` |

## DESIGN.md rules

`DESIGN.md` is the living, technology-independent architecture for rebuilding
WebUI. Update it in the same change only when modifying:

- Subsystem responsibilities and data flow
- Build-time versus request-time ownership
- Cross-layer design invariants and compatibility boundaries
- Technology choices essential to reproducing the system

Revise existing explanations instead of appending incident histories. Exact
schemas, signatures, error codes, SSR marker layouts, metadata tuples, and
regression details belong in the defining source and tests; preserve focused
subsystem and cross-language contracts in internal `specs/` references. A
field or API change alone is not an architectural change. If an architectural
claim and the code disagree, reconcile them.

## docs/ rules

Update `docs/` in the same commit when the change is user-visible:

- CLI usage or flags changed
- Template syntax or rendering output changed
- Integration behavior that external developers depend on
- New features or removed features

### Public API boundary

Developer documentation (`docs/`, crate/package READMEs, and
`docs/ai.md`) documents only supported public APIs and externally
observable contracts. Every addition must map to at least one public entry
point:

- an exported Rust, Node, WASM, FFI, or package API;
- a CLI command, flag, configuration field, or supported identifier;
- supported template/component authoring syntax;
- a documented protocol or integration contract.

Do not document private or `pub(crate)` items, internal callbacks, intermediate
representations, cache algorithms, implementation sequencing, regression-test
details, or dependency-specific workarounds in developer docs. Put system-level
architecture in `DESIGN.md`, subsystem technical contracts in `specs/`,
and implementation invariants and local rationale near code and tests.

Rust `///` documentation comments are for exported public APIs only. Use `//`
sparingly for non-public implementation rationale. Before finishing, audit the
documentation diff against the public exports and remove text that has no public
entry point.

Do not update user-facing docs for internal implementation details, regression
tests, refactors, or bug fixes that only restore already-documented behavior.
Every addition must help developers author, configure, debug, or integrate a
WebUI application. Do not add release-note-style implementation observations
to reference docs.

Keep protocol internals out of general user docs. The `docs/ai.md` file is the single-page AI reference and should be kept in sync with all other docs.

`docs/ai.md` is authoring-first by design. Keep deep reference material (full CLI flag tables, error-code lists, per-language integration snippets) in its canonical page and link to it from `docs/ai.md` rather than duplicating it there.

`ai/SKILL.md` is only the stable loader. Update `docs/ai.md` for authoring changes,
not the loader or the generated `packages/webui/ai.md`. The package prepack step
refreshes the generated reference; ordinary builds do not write it.

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
