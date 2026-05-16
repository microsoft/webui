# Embedded Fragment Rendering

WebUI's integrations are designed around rendering a full page in one shot. There's a second, smaller story that the framework supports just as well but isn't yet called out in these docs: rendering a **single named fragment** from a host that is not itself a WebUI app. This page covers that.

## When you'd want this

- You have an existing app in another framework (Express + React, Tanstack Start, a Rust web server with a hand-written template engine, etc.) and want to adopt WebUI for one component - say, a citation group inside a chat stream - without rewriting the rest of the page.
- You're streaming HTML from a non-WebUI host and want to inject one rendered fragment into the stream as part of a larger response.

Both of these are supported today via the `entry` option on the render call. The host stays in charge of the surrounding HTML; WebUI renders the named fragment and returns (or streams) just its bytes.

## Recipe: Node host

Use `renderStream` when you want the rendered fragment to interleave into an existing response stream, or `render` when you want the full string and you'll stitch it in yourself.

```js
import { renderStream } from '@microsoft/webui';
import { readFileSync } from 'node:fs';

const protocol = readFileSync('./dist/protocol.bin');

// Inside an existing request handler:
function streamCitationGroup(res, citations) {
  res.write('<section class="message-body">');
  renderStream(
    protocol,
    { citations },
    (chunk) => res.write(chunk),
    { entry: 'citation-group.html' }, // <- the fragment to render
  );
  res.write('</section>');
}
```

`entry` is the fragment ID (the relative HTML filename inside `appDir` at build time). You can have many entries in one protocol and pick the one the host needs per call. Only the templates reachable from `entry` are walked; nothing else in the protocol is rendered.

### How fragments are keyed

The protocol stores two distinct maps and the embedded recipe only addresses the first:

- `fragments: map<string, FragmentList>` - keyed by the relative path of the source HTML file inside `appDir`. `appDir/index.html` becomes the `"index.html"` fragment; `appDir/widgets/citation-group.html` becomes the `"widgets/citation-group.html"` fragment. `entry` always selects from this map.
- `components: map<string, ComponentData>` - keyed by tag name. Populated by the active parser plugin (for example, `fast-v3` registers each `<f-template name="...">` block under its `name` attribute). These are referenced *by* fragments via tag use; they cannot be the `entry` of a render call directly.

In practice this means a FAST-3 host that wants to render exactly one custom element from a single embedded call needs a thin wrapper HTML file, not the `<f-template>` block alone. For example, given `src/citation-group/citation-group.html` containing a `<f-template name="bebop-citation-group">…</f-template>` block, also create a wrapper:

```html
<!-- src/citation-group.html -->
<bebop-citation-group></bebop-citation-group>
```

Build with `webui build src --plugin fast-v3 --out dist` and pass `entry: 'citation-group.html'` to `render` or `renderStream` (or `RenderOptions::new("citation-group.html", "/")` in Rust). The wrapper resolves the component reference, the plugin emits the hydration markers, and the host gets back the bytes for that one element.

If you pass an `entry` that does not match a key in `fragments`, the call returns `HandlerError::MissingFragment(name)` in Rust and throws an analogous error from the Node API. You can list the keys in a built protocol with `webui inspect dist/protocol.bin`.

If you'd rather get the rendered string back and inject it as a `${html}` substitution into your own template engine, swap `renderStream` for `render`:

```js
import { render } from '@microsoft/webui';

const html = render(protocol, { citations }, { entry: 'citation-group.html' });
// ... pass `html` to your existing template ...
```

## Recipe: Rust host

The Rust integration exposes the same shape via `WebUIHandler::handle` plus a custom `ResponseWriter`. The writer is where you decide what to do with each chunk - write it to an `axum::body::Bytes` channel, push it onto a `Vec<u8>`, send it down a websocket frame, whatever the host expects.

```rust
use std::{fs, sync::Arc};
use webui::{HandlerResult, RenderOptions, ResponseWriter, WebUIHandler};

struct StringWriter(String);
impl ResponseWriter for StringWriter {
    fn write(&mut self, content: &str) -> HandlerResult<()> {
        self.0.push_str(content);
        Ok(())
    }
    fn end(&mut self) -> HandlerResult<()> { Ok(()) }
}

fn render_citation_group(
    handler: &WebUIHandler,
    protocol: &[u8],
    state: &serde_json::Value,
) -> String {
    let mut writer = StringWriter(String::new());
    let options = RenderOptions::new("citation-group.html", "/");
    handler
        .handle(protocol, state, options, &mut writer)
        .expect("render failed");
    writer.0
}
```

The handler is `Send + Sync` (see [Thread safety](./rust#thread-safety)), so the typical pattern is to construct it once at startup, wrap it in `Arc`, and call `handle` from any request task with a fresh writer.

## Fragments and routing

The `requestPath` argument is independent of `entry`. If your fragment contains a `<route>` directive - for example, a "currently selected tab" pattern - pass the relevant path so the inner route matcher fires. If the fragment is route-free, pass `"/"`. Non-matching routes inside the fragment render hidden-and-empty exactly as they would in a full-page render.

## What you don't get

This recipe deliberately skips the things a full-page WebUI host gives you for free:

- **The `webui-framework` client runtime.** If your fragment uses interactive components, you need to load that runtime in the host page yourself (`<script type="module" src="webui-framework.js">`).
- **Module CSS deduplication across fragments.** Each fragment render is independent. If the host page renders two WebUI fragments and both use the same component, you'll get the `<style type="module">` tag twice unless you dedupe at the host layer.
- **Inventory tracking for client-side navigation.** `renderPartial` and `renderComponentTemplates` (Node) and their Rust equivalents are designed for full WebUI navigation. The embedded recipe just renders the fragment once and walks away.

These are intentional trade-offs of embedding-mode. If you find yourself wanting them, the integration has likely outgrown a single fragment and is ready to graduate to one of the full-page recipes in [Rust](./rust), [Node / Bun / Deno](./node), or [Electron](./electron).
