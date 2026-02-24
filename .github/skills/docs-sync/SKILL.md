---
name: docs-sync
description: Keep user-facing docs and DESIGN specification aligned with behavior and API changes.
---

# Docs Synchronization Workflow

Use this skill for user-visible behavior changes or API/contract changes.

## DESIGN.md (spec) requirements

Update `DESIGN.md` in the same change when modifying:

- public APIs
- protocol fields
- behavioral contracts
- error variants

Treat `DESIGN.md` as the implementation specification.

## docs/ requirements

Update `docs/` in the same change when behavior is user-visible:

- CLI usage changes
- template syntax/state/render output changes
- integration behavior that external users depend on

Keep protocol internals out of general user docs unless placed in advanced protocol documentation.

## Optional validation

When docs are changed substantially, validate with:

```bash
cd docs && pnpm build
```
