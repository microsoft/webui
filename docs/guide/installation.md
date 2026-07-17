# Installation

WebUI Framework can be installed and used with various environments and languages. This guide covers the most common installation methods.

There are two ways to install the WebUI build toolchain: as an **npm package** for JavaScript and TypeScript projects, or as a **Rust crate** for Rust projects. Both ship the same compiler and produce the same protocol output. Pick whichever fits your stack.

## npm

The `@microsoft/webui` npm package gives you:

- **`npx webui build`** - the CLI for building templates into protocols
- **`import { build, Protocol } from '@microsoft/webui'`** - a programmatic API for Node.js
- **Native performance** via platform-specific binaries (no compilation required)

<webui-press-tabs>
<webui-press-tab slot="tab" active>npm</webui-press-tab>
<webui-press-tab slot="tab">yarn</webui-press-tab>
<webui-press-tab slot="tab">pnpm</webui-press-tab>
<webui-press-tab-panel active>

```bash
npm install @microsoft/webui
```

</webui-press-tab-panel>
<webui-press-tab-panel>

```bash
yarn add @microsoft/webui
```

</webui-press-tab-panel>
<webui-press-tab-panel>

```bash
pnpm add @microsoft/webui
```

</webui-press-tab-panel>
</webui-press-tabs>

### Configure package.json

A typical project setup:

```json
{
  "scripts": {
    "build": "webui build ./src --out ./dist --plugin=webui",
    "dev": "webui serve ./src --state ./data/state.json --plugin=webui --watch"
  },
  "dependencies": {
    "@microsoft/webui": "latest",
    "@microsoft/webui-framework": "latest"
  }
}
```

Run the development server with `npm run dev` and build for production with `npm run build`.

### Cross-Platform Support

The npm package uses platform-specific optional dependencies to deliver native binaries. Supported platforms are installed automatically - no Rust toolchain required.

## Rust

Rust users can install the CLI directly from crates.io:

```bash
cargo install microsoft-webui-cli
```

Then build your app:

```bash
webui build ./my-app --out ./dist
```

See the [CLI Reference](/guide/cli/) for full usage details.

## .NET

The managed .NET binding is packaged as `Microsoft.WebUI`:

```bash
dotnet add package Microsoft.WebUI
```

It targets .NET 8 and .NET 9. The package restores platform-specific `Microsoft.WebUI.Runtime.*` packages transitively, and .NET selects the matching native asset. Release builds stage `.nupkg` and `.snupkg` artifacts with Source Link and repository metadata for downstream signing and publishing. NuGet.org publishing is not automatic until an approved Microsoft-certificate signing path is available for `.nupkg` packages.

Prepare `protocol.bin` once for repeated rendering:

```csharp
using Microsoft.WebUI;

using var protocol = new Protocol(
    File.ReadAllBytes("dist/protocol.bin"));
using var handler = new WebUIHandler("webui");

string html = handler.Render(
    protocol,
    """{"title":"Home"}""",
    "index.html",
    "/");
```

`Protocol` is thread-safe and owns the decoded protocol plus reusable indices.
Keep it alive for the server lifetime and dispose it during shutdown.

---

The packages below are client-side runtime libraries. They are installed from npm regardless of whether your build toolchain is npm or Rust, since they ship as JavaScript that runs in the browser.

## WebUI Framework (Client-Side Interactivity)

For interactive Web Components with Islands Architecture, install the framework runtime:

<webui-press-tabs>
<webui-press-tab slot="tab" active>npm</webui-press-tab>
<webui-press-tab slot="tab">yarn</webui-press-tab>
<webui-press-tab slot="tab">pnpm</webui-press-tab>
<webui-press-tab-panel active>

```bash
npm install @microsoft/webui-framework
```

</webui-press-tab-panel>
<webui-press-tab-panel>

```bash
yarn add @microsoft/webui-framework
```

</webui-press-tab-panel>
<webui-press-tab-panel>

```bash
pnpm add @microsoft/webui-framework
```

</webui-press-tab-panel>
</webui-press-tabs>

This gives you:
- **`WebUIElement`** base class for interactive Web Components
- **`@attr`** and **`@observable`** decorators for reactive state
- Automatic SSR hydration with zero manual DOM reading
- Path-indexed targeted updates for minimal DOM mutations

<webui-blockquote appearance="tip" title="Not every app needs this" icon="💡">

If your pages are purely informational and never receive client-side state updates, you only need `@microsoft/webui` for building and rendering. Load the framework when components need browser-applied state or soft navigation. Add a same-named component module only for events, lifecycle code, decorators, or imperative APIs.

</webui-blockquote>

## Client-Side Router (Optional)

For single-page navigation with client-side route transitions, install the router package:

<webui-press-tabs>
<webui-press-tab slot="tab" active>npm</webui-press-tab>
<webui-press-tab slot="tab">yarn</webui-press-tab>
<webui-press-tab slot="tab">pnpm</webui-press-tab>
<webui-press-tab-panel active>

```bash
npm install @microsoft/webui-router
```

</webui-press-tab-panel>
<webui-press-tab-panel>

```bash
yarn add @microsoft/webui-router
```

</webui-press-tab-panel>
<webui-press-tab-panel>

```bash
pnpm add @microsoft/webui-router
```

</webui-press-tab-panel>
</webui-press-tabs>

The router works with both WebUI Framework (`@microsoft/webui-framework`) and `@microsoft/fast-element` 3.x components. It's a separate package because it's only needed for apps with client-side navigation.

See the [Routing guide](/guide/concepts/routing) for setup and usage.
