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

// Create a handler (optionally with a plugin like "fast" for FAST-HTML)
using var handler = new WebUIHandler("fast");

// Load pre-compiled protocol binary (from `webui build`)
byte[] protocol = File.ReadAllBytes("app.webui");

// Render with different state each time
var html = handler.Render(protocol, """{"user": "Alice"}""", "index.html", "/");
```

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
