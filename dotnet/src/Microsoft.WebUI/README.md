# Microsoft.WebUI

High-performance server-side rendering for .NET — no JavaScript runtime required.

WebUI separates static and dynamic content at build time into a binary protocol that enables fast rendering in any host language. This package provides .NET bindings to the WebUI native rendering engine.

## Quick Start

```csharp
using Microsoft.WebUI;

// Load pre-compiled protocol binary (from `webui build`)
using var protocol = new Protocol(File.ReadAllBytes("app.webui"));
using var handler = new WebUIHandler("webui");

// Render with different state each time
var html = handler.Render(protocol, """{"user": "Alice"}""", "index.html", "/");
```

`Protocol` is thread-safe and owns the decoded protocol plus reusable indices.
Keep it alive for the server lifetime. Refer to the WebUI documentation for the
available plugin identifiers.

## Client-Side Navigation (Partial Responses)

When the client navigates via the WebUI Router, your server returns a JSON partial instead of full HTML. Use `RenderPartial` — one call produces the complete response with state, templates, inventory, path, and matched route chain:

```csharp
app.MapGet("/users/{id}", (HttpContext ctx, string id) =>
{
    var state = new { name = GetUser(id).Name };
    var stateJson = JsonSerializer.Serialize(state);

    if (ctx.Request.Headers.Accept.Contains("application/json"))
    {
        // Client-side navigation — return JSON partial (no assembly required)
        var inventoryHex = ctx.Request.Headers["X-WebUI-Inventory"].FirstOrDefault() ?? "";
        var json = protocol.RenderPartial(stateJson, "index.html", ctx.Request.Path, inventoryHex);
        return Results.Content(json, "application/json");
    }

    // Full SSR — return complete HTML page
    var html = handler.Render(protocol, stateJson, "index.html", ctx.Request.Path);
    return Results.Content(html, "text/html");
});
```

The response is a JSON string — pipe it directly to the HTTP response. No deserialization needed.

`protocol.RenderComponentTemplates(tags, inventoryHex)` returns the template
payload for on-demand component loading. `protocol.Tokens()` returns CSS token
names in build order.

## Installation

```bash
dotnet add package Microsoft.WebUI
```

The managed package depends on all supported `Microsoft.WebUI.Runtime.<rid>` packages. NuGet restores those native runtime packages transitively, and .NET selects the matching `runtimes/<rid>/native` asset for your platform.

### Supported Platforms

| Runtime | Package |
|---------|---------|
| Windows x64 | `Microsoft.WebUI.Runtime.win-x64` |
| Windows ARM64 | `Microsoft.WebUI.Runtime.win-arm64` |
| Linux x64 | `Microsoft.WebUI.Runtime.linux-x64` |
| Linux ARM64 | `Microsoft.WebUI.Runtime.linux-arm64` |
| macOS x64 | `Microsoft.WebUI.Runtime.osx-x64` |
| macOS ARM64 | `Microsoft.WebUI.Runtime.osx-arm64` |

### Package Metadata

Packed NuGet artifacts include this README, repository metadata, Source Link, a package license URL with license acceptance required, release notes links, discoverability tags, the `© Microsoft Corporation. All rights reserved.` notice, and `.snupkg` symbol packages. Release workflows stage `.nupkg` and `.snupkg` files; nuget.org publishing remains manual/externally tracked until ESRP supports automated NuGet publishing for this project. Before publishing, staged packages and Authenticode-signable contents must be signed with a Microsoft certificate through the approved signing process.

### Manual Native Library Path

If you need to point to a custom build of the native library:

```bash
export WEBUI_LIB_PATH=/path/to/directory   # directory containing libwebui_ffi.*
# or
export WEBUI_LIB_PATH=/path/to/libwebui_ffi.dylib  # direct file path
```

## Building from Source

```bash
# Build the native FFI library
cargo build --release -p microsoft-webui-ffi

# Build and test the .NET package
cargo xtask dotnet
```

## License

MIT. NuGet package metadata uses © Microsoft Corporation. All rights reserved.
