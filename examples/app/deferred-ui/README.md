# Deferred popover and dialog

A runnable example of inline `<if defer>` bodies inside a native `popover` and
`<dialog>`. Each overlay is a component; its deferred body stays inline rather
than becoming another component or a route.

| Component | Owns |
| --- | --- |
| `deferred-workspace` | Reading list, shared slow-data control, cache control, and applied view preferences |
| `deferred-popover` | Native `popovertarget` controls, CSS anchor positioning, and the deferred `activity` body |
| `deferred-dialog` | Native `showModal()` / `close()`, draft preferences, and the deferred `settings` body |

The workspace passes boolean attributes to its children. The dialog emits
`preferences-applied`; both overlays can emit `overlay-notice`. Each component
owns its request cancellation and local state. `src/deferred-state.ts` shares
one resource loader, not overlay state.

```bash
# From the repository root
pnpm install
cargo xtask dev deferred-ui
```

Open `http://127.0.0.1:3011`. The small Node state API runs on port `3021`;
`webui serve` owns SSR, templates, inventory, and the two-phase response.

- Hover or focus **Recent activity** or **View settings** to preload resources.
- Clicking fetches fresh scoped state, even when the templates are already loaded.
  The overlay opens only after all its content is ready.
- Enable **Simulate slow data** to observe metadata-first delivery and cancellation.
- While loading, press Escape or click **Cancel activity load** / **Cancel settings
  load**. Loading and retry feedback stay beside the trigger, outside the overlay.
- Escape or Close removes the view and clears its request state. The popover also
  supports native light-dismiss.
- Preferences affect only the current page. The content is illustrative sample data.
- `/?open=popover` and `/?open=dialog` demonstrate initially active SSR bodies.
- **Clear template cache** releases unused cached definitions without removing
  the live workspace; opening again reacquires what is needed.

Only the triggers, empty native containers, and their external loading/error
feedback are eager. Every element inside the popover and dialog, including
headings, close buttons, and form controls, belongs to its deferred body.
`activity` and `settings` use the shared hydration-agnostic loader from
`@microsoft/webui-router/deferred.js` with the opt-in NDJSON decoder. The
framework's deferred entry only provides rendering in the existing owner.
Endpoints resolve against the document's base URL so the same app works in the
demo catalog's subdirectory.

Keep `defer` on both named conditions. An ordinary `<if id="activity">` is not a
deferred resource; its loader request returns `deferred-not-found`.

```bash
pnpm --dir examples/app/deferred-ui build
pnpm --dir examples/app/deferred-ui test
```

The focused tests cover initial omission, preload without mounting, warm
inventory with fresh state, native anchoring and dismissal, component attributes
and events, cancellation, retry, SSR, collection/reopen, and narrow viewports.
