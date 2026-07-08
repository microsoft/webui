// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Hydration-mismatch diagnostic (issue #379).
 *
 * A reactive write that runs while an element is connected but before hydration
 * finishes — a `@observable` field initializer, the `constructor`, or code
 * before `super.connectedCallback()` — is dropped by `TemplateElement.$update`'s
 * pre-ready guard: the backing field changes, but the server-rendered DOM is
 * trusted and left untouched. That leaves the element's own observable
 * disagreeing with its DOM — silently, and inconsistently with client-side
 * rendering.
 *
 * The framework does NOT reconcile trusted SSR content: patching the DOM would
 * only repair the post-hydration state while leaving the first server paint
 * wrong, and it would erode the SSR-trust invariant (#286). Instead
 * `TemplateElement` records the pre-ready write paths and, once hydrated, calls
 * into this module to report any that disagree with the DOM — the same
 * hydration-mismatch signal React, Vue, Svelte, and Solid emit.
 *
 * This module is intentionally free of element internals: the comparisons are
 * pure functions over binding objects plus a small resolver context, so they
 * unit-test in isolation (see `hydration-mismatch.test.ts`) and keep the core
 * `template-element.ts` focused on lifecycle wiring. The comparisons are
 * read-only — they never mutate the DOM.
 */

import type { CompiledAttrPart } from './template.js';
import {
  ATTR_KIND_BOOLEAN,
  ATTR_KIND_COMPLEX,
  ATTR_KIND_TEMPLATE,
  type AttrBinding,
  type CondBinding,
  type RepeatBinding,
  type ScopeFrame,
  type TextBinding,
} from './element/types.js';

/**
 * The slice of `TemplateElement` the comparators need to evaluate a binding's
 * current (post-write) value. Supplied by the element so this module never
 * reaches into private state.
 */
export interface MismatchContext {
  /** Resolver handed to compiled condition functions (`condition[0]`). */
  resolver: (path: string, scope?: unknown) => unknown;
  /** Resolve a template-part list (`data-x="{{a}}-{{b}}"`) to its string value. */
  resolveParts: (parts: CompiledAttrPart[], scope?: ScopeFrame) => string;
  /** Resolve a single property path to its current value. */
  resolveValue: (path: string, scope?: ScopeFrame) => unknown;
}

/** Bindings for one observable root, grouped by kind (the path-index entry). */
export interface PathBindings {
  texts: TextBinding[];
  attrs: AttrBinding[];
  conds: CondBinding[];
  repeats: RepeatBinding[];
}

/**
 * Entry point invoked by `TemplateElement` through a dynamic `import()`.
 *
 * Compares each recorded pre-ready write path against the SSR DOM and, if any
 * diverged, emits the hydration-mismatch warning once. Routing the element
 * through this function keeps every comparator and the message string in this
 * module, reached only via that dynamic import — so a production bundler drops
 * the whole module when `__WEBUI_DEV__` folds `DEV` to `false` (see
 * `template-element.ts`). The comparison is read-only; it never mutates the DOM.
 */
export function reportHydrationMismatch(
  tag: string,
  writes: ReadonlySet<string>,
  index: ReadonlyMap<string, PathBindings>,
  ctx: MismatchContext,
): void {
  const mismatched: string[] = [];
  for (const path of writes) {
    const entry = index.get(path);
    if (entry && bindingsDisagreeWithDom(entry, ctx)) mismatched.push(path);
  }
  if (mismatched.length !== 0) warnHydrationMismatch(tag, mismatched);
}

/**
 * True when any binding for a recorded pre-ready write disagrees with the DOM.
 * Stops at the first disagreement — the caller only needs to know whether the
 * observable diverged, not every binding that did.
 */
export function bindingsDisagreeWithDom(entry: PathBindings, ctx: MismatchContext): boolean {
  const { texts, attrs, conds, repeats } = entry;
  for (let i = 0; i < texts.length; i++) if (textDiffersFromDom(texts[i], ctx)) return true;
  for (let i = 0; i < attrs.length; i++) if (attrDiffersFromDom(attrs[i], ctx)) return true;
  for (let i = 0; i < conds.length; i++) if (condDiffersFromDom(conds[i], ctx)) return true;
  for (let i = 0; i < repeats.length; i++) if (repeatDiffersFromDom(repeats[i], ctx)) return true;
  return false;
}

/** Text binding: compare the resolved value against the rendered text. */
export function textDiffersFromDom(b: TextBinding, ctx: MismatchContext): boolean {
  // Raw `{{{expr}}}` bindings render via innerHTML, which the browser
  // re-serializes on read (quote style, entity/attribute normalization, void
  // elements), so a value that truly matches SSR can compare unequal. Skip
  // them rather than emit a false mismatch.
  if (b.raw) return false;
  let expected: string;
  if (b.parts) {
    expected = ctx.resolveParts(b.parts, b.scope);
  } else if (b.path) {
    const raw = ctx.resolveValue(b.path, b.scope);
    expected = raw == null ? '' : String(raw);
  } else {
    return false;
  }
  return b.node.data !== expected;
}

/** Attribute binding: compare per kind, skipping cases SSR legitimately owns. */
export function attrDiffersFromDom(b: AttrBinding, ctx: MismatchContext): boolean {
  const el = b.element;
  switch (b.kind) {
    // Complex `:prop` bindings carry parent-delayed object data that a child
    // legitimately hydrates before the parent sets it — SSR is trusted (#286).
    case ATTR_KIND_COMPLEX:
      return false;
    case ATTR_KIND_BOOLEAN:
      return el.hasAttribute(b.name) !== !!b.condition![0](ctx.resolver, b.scope);
    case ATTR_KIND_TEMPLATE:
      return (el.getAttribute(b.name) ?? '') !== ctx.resolveParts(b.parts!, b.scope);
    default: {
      // Form-control properties diverge from their attribute after user
      // interaction, so an attribute comparison would be misleading.
      if (b.name === 'value' || b.name === 'checked' || b.name === 'selected') return false;
      const v = ctx.resolveValue(b.path!, b.scope);
      return (el.getAttribute(b.name) ?? '') !== (v == null ? '' : String(v));
    }
  }
}

/**
 * Conditional binding: compare rendered PRESENCE, not the raw value. A pre-ready
 * write that changes the value but not the condition result (e.g. `count` 5→3
 * under `count > 0`) leaves the DOM correct and must not warn.
 */
export function condDiffersFromDom(c: CondBinding, ctx: MismatchContext): boolean {
  return (c.instance != null) !== !!c.condition[0](ctx.resolver, c.scope);
}

/**
 * Repeat binding: compare the collection length against the rendered item
 * count. Length is an intentional proxy — a same-length content or order change
 * is not flagged, which keeps this cold-path check O(1) per repeat. Non-array
 * values coerce to length 0, mirroring the renderer (`element/diff.ts` and the
 * hydration path both treat non-arrays as empty), so the comparison stays
 * consistent with what was actually rendered.
 */
export function repeatDiffersFromDom(r: RepeatBinding, ctx: MismatchContext): boolean {
  const resolved = ctx.resolveValue(r.collection, r.scope);
  const expectedLength = Array.isArray(resolved) ? resolved.length : 0;
  return expectedLength !== r.instances.length;
}

/** Build the developer-facing warning text for a set of diverged observables. */
export function formatHydrationMismatch(tag: string, paths: string[]): string {
  const list = paths.map((p) => `"${p}"`).join(', ');
  return (
    `[WebUI] Hydration mismatch on <${tag}>: ` +
    `${list} changed at or before super.connectedCallback() to a value that ` +
    `differs from the server-rendered DOM. The DOM keeps the server value ` +
    `(SSR content is trusted), so the element's state and DOM disagree. ` +
    `Add the value to the SSR state, or assign it after super.connectedCallback().`
  );
}

/**
 * `(tag, path-set)` pairs already warned about in this document. Deduping on
 * the exact set of diverged paths — not just the tag — keeps a repeated
 * instance of the same bug quiet while still letting a genuinely different
 * later mismatch on the same component surface.
 */
const warnedMismatches = new Set<string>();

/** Emit the hydration-mismatch warning once per `(tag, diverged-path-set)`. */
export function warnHydrationMismatch(tag: string, paths: string[]): void {
  const key = `${tag}\u0000${[...paths].sort().join('\u0000')}`;
  if (warnedMismatches.has(key)) return;
  warnedMismatches.add(key);
  console.warn(formatHydrationMismatch(tag, paths));
}

/**
 * Clear the dedup cache. Test-only: warnings dedupe for the lifetime of the
 * document, so tests call this to assert warning behavior in isolation.
 */
export function resetHydrationMismatchWarnings(): void {
  warnedMismatches.clear();
}
