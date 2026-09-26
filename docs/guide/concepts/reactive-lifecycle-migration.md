# Migrate Reactive Property Callbacks

WebUI now calls one `propertiesChanged(changes, firstChange)` hook after a
component is connected and hydrated instead of invoking `nameChanged(old,
new)` from each decorated property's setter. Update authored components that
use `@attr` or `@observable` callbacks before upgrading the framework.

```typescript
// Before: fired during field initialization, before refs and internals existed.
checkedChanged(): void {
  if (!this.isConnected || !this.internals) return;
  this.syncState();
}

// After: one initial pass plus coalesced updates, with refs ready.
protected override propertiesChanged(
  changes: ReadonlyMap<string, unknown>,
  firstChange: boolean,
): void {
  if (firstChange || changes.has('checked')) {
    this.internals.ariaChecked = String(this.checked);
    this.internals.setFormValue(this.checked ? 'on' : null);
  }
}
```

Initialize `ElementInternals` in the constructor with `attachInternals()`;
declare your `w-ref` fields as usual. The first callback runs after this
component's own refs and rendered DOM are ready, with the final initial
values from field initializers, HTML attributes, SSR state, and parent
properties. It runs once per instance, not once per assignment or reconnect.
`firstChange` is `true` for this pass, and `changes` holds `undefined` as the
old value of each initialized decorated property. Read its current value from
the element.

Remove `isConnected` or uninitialized-internals guards that only protected
old constructor-time calls. Consolidate `syncState()` override chains into
one hook when multiple properties determine the same host state. Preserve
guards for genuinely optional browser features, lazy descendants, or external
resources that can still be unavailable.

After hydration, property values and `@attr` host reflection change
synchronously. Template bindings and `propertiesChanged` are batched in a
microtask; code needing their results immediately can call `$flushUpdates()`.
An external attribute change synchronously updates its decorated property.
Assigning decorated properties while disconnected stores the value and runs
the effect after reconnection. Existing SSR host attributes take precedence
over projected `@attr` state.

Old `nameChanged` methods are not invoked. Development builds throw an
actionable error when a decorated property still has a matching legacy
method; production builds omit this migration check. Migrate every such
method before deploying. Pure, constructor-time calculations belong in field
initializers or getters; work that needs refs belongs in `propertiesChanged`
or the once-only `hydratedCallback()`.
