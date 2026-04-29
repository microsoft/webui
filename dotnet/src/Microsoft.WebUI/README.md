# Microsoft.WebUI

High-performance server-side rendering for .NET — no JavaScript runtime required.

WebUI separates static and dynamic content at build time into a binary protocol that enables fast rendering in any host language. This package provides .NET bindings to the WebUI native rendering engine.

## Quick Start

```csharp
using Microsoft.WebUI;

// One-shot render (parse + render in a single call)
var html = "<div>Hello, {{name}}!</div>";
var json = """{"name": "World"}""";
var result = WebUIRenderer.RenderHtml(html, json);
// result: "<div>Hello, World!</div>"
```

## Handler API (Higher Performance)

For repeated renders with pre-compiled protocol data, use the `WebUIHandler`:

```csharp
using Microsoft.WebUI;

// Create a handler (optionally with "fast-v3" for FAST 3 hydration)
using var handler = new WebUIHandler("fast-v3");

// Load pre-compiled protocol binary (from `webui build`)
byte[] protocol = File.ReadAllBytes("app.webui");

// Render with different state each time
var html = handler.Render(protocol, """{"user": "Alice"}""", "index.html", "/");
```

`"fast"` and `"fast-v2"` remain available only for deprecated FAST 2 compatibility. Use `"fast-v3"` for FAST 3.

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
        var json = handler.RenderPartial(protocol, stateJson, "index.html", ctx.Request.Path, inventoryHex);
        return Results.Content(json, "application/json");
    }

    // Full SSR — return complete HTML page
    var html = handler.Render(protocol, stateJson, "index.html", ctx.Request.Path);
    return Results.Content(html, "text/html");
});
```

The response is a JSON string — pipe it directly to the HTTP response. No deserialization needed.

## Installation

```bash
dotnet add package Microsoft.WebUI
```

The correct native runtime package for your platform is automatically resolved by NuGet.

### Supported Platforms

| Runtime | Package |
|---------|---------|
| Windows x64 | `Microsoft.WebUI.Runtime.win-x64` |
| Windows ARM64 | `Microsoft.WebUI.Runtime.win-arm64` |
| Linux x64 | `Microsoft.WebUI.Runtime.linux-x64` |
| Linux ARM64 | `Microsoft.WebUI.Runtime.linux-arm64` |
| macOS x64 | `Microsoft.WebUI.Runtime.osx-x64` |
| macOS ARM64 | `Microsoft.WebUI.Runtime.osx-arm64` |

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
cargo build --release -p webui-ffi

# Build and test the .NET package
cargo xtask dotnet
```

## License

MIT
