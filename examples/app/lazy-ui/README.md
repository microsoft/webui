# Lazy popover and dialog

A runnable example of inline `<if lazy>` bodies inside a native `popover` and
`<dialog>`. Each overlay is a component; its lazy body stays inline rather
than becoming another component or a route.

| Component | Owns |
| --- | --- |
| `lazy-workspace` | Reading list, shared slow-data control, cache control, and applied view preferences |
| `lazy-popover` | Native `popovertarget` controls, CSS anchor positioning, and the lazy activity body |
| `lazy-dialog` | Native `showModal()` / `close()`, draft preferences, and the lazy settings body |

The workspace passes boolean attributes to its children, and the dialog emits
`preferences-applied`. Each overlay owns its request cancellation and local
state. `src/state-api.ts` is the only data helper: a plain `fetch()` of the
overlay's state from the sample API.

```bash
# From the repository root
pnpm install
cargo xtask dev lazy-ui
```

Open `http://127.0.0.1:3011`. `webui serve` renders the page and serves the
build's template packs from `_webui/packs/`. The small Node API on port `3021`
returns only state, at `/api/activity` and `/api/settings`.

- Hover or focus **Recent activity** or **View settings**: the framework
  prefetches that overlay's template pack. No state is requested and nothing
  renders.
- Clicking fetches fresh state and the pack in parallel, activates the body,
  and waits for `this.$rendered()` before showing the overlay.
- Enable **Simulate slow data** to observe loading feedback and cancellation.
- While loading, press Escape or click **Cancel activity load** / **Cancel settings
  load**. Loading and retry feedback stay beside the trigger, outside the overlay.
- Escape or Close removes the body and clears its request state. The popover
  also supports native light-dismiss. At the next idle period the framework
  releases the closed body's template; reopening reuses it if it is still in
  memory, or reloads the pack from the HTTP cache.
- Preferences affect only the current page. The content is illustrative sample data.
- `/?open=popover` and `/?open=dialog` render an initially active body on the
  server, with its template inline and no pack request.
- **Clear template cache** releases unused cached definitions without removing
  the live workspace; opening again reacquires what is needed.

Only the triggers, empty native containers, and their external loading/error
feedback are eager. Every element inside the popover and dialog, including
headings, close buttons, and form controls, belongs to its lazy body.
`src/index.ts` imports `@microsoft/webui-framework/lazy.js` once; the template
needs no identifier. Pack and state URLs resolve against the document's base
URL, so the same app works in the demo catalog's subdirectory.

```bash
pnpm --dir examples/app/lazy-ui build
pnpm --dir examples/app/lazy-ui test
```

The tests cover initial omission, intent prefetch without mounting, warm packs
with fresh state, native anchoring and dismissal, component attributes and
events, cancellation, pack and state failures with retry, SSR, cache
collection and reopen, the base path, and narrow viewports.
