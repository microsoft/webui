---
title: "Go"
description: "Import the official Go binding, configure the native SDK, and render compiled templates with safe concurrent ownership."
---

# Go

The `github.com/microsoft/webui/go/v3` module renders through WebUI's native
Rust C ABI. It has no third-party Go dependencies and does not run JavaScript
to render HTML. The package owns native handles; your application owns HTTP,
request state, and public asset delivery.

## Installation

You need Go 1.25 or newer, cgo, a compatible C compiler, and the WebUI native
SDK matching the Go module's version, operating system, and architecture.

```bash
go get github.com/microsoft/webui/go/v3
```

Go module versions use the repository's `go/v<version>` tags, independently
of the root `v<version>` tags. An unpublished version is not available from
the public Go proxy until that module tag has been published.

Download the matching standalone `webui-sdk-<version>-<platform>.tar.gz` and
its `.sha256` file from the [release assets](https://github.com/microsoft/webui/releases).
Verify the archive against the published checksum before extracting it into
an SDK directory. The extracted root contains `include/`, `lib/`,
`sdk.json`, and license notices. npm is not required for this setup.

The native packages target Windows, macOS, and GNU/Linux on x64 and ARM64.
Linux requires glibc; musl is not supported. Cross-compilation additionally
requires the target's SDK and C toolchain.

For macOS or Linux, supply the header and library search paths when building:

```bash
export WEBUI_SDK_DIR=/absolute/path/to/sdk
export CGO_ENABLED=1
export CGO_CFLAGS="\"-I$WEBUI_SDK_DIR/include\""
export CGO_LDFLAGS="\"-L$WEBUI_SDK_DIR/lib\""
go build ./...
```

The package selects `-lwebui_ffi` itself. On Windows it selects the SDK's
`webui_ffi.dll.lib` import library; use a cgo-compatible Windows C toolchain.
For PowerShell:

```powershell
$env:WEBUI_SDK_DIR = 'C:\sdks\webui'
$env:CGO_ENABLED = '1'
$env:CGO_CFLAGS = '"-I' + $env:WEBUI_SDK_DIR + '\include"'
$env:CGO_LDFLAGS = '"-L' + $env:WEBUI_SDK_DIR + '\lib"'
$env:PATH = "$env:WEBUI_SDK_DIR\lib;$env:PATH"
go build ./...
```

The operating-system loader must also find the shared library at runtime.
Use your deployment's normal library search path, or configure
`DYLD_LIBRARY_PATH` on macOS, `LD_LIBRARY_PATH` on Linux, or `PATH` on
Windows. Keep the native library available for the process's entire lifetime.
The package never downloads or builds a native library automatically.

## Build once, render repeatedly

Compile templates with the same WebUI release:

```bash
webui build ./src --out ./dist --plugin webui
```

Load the protocol once at startup and reuse its renderer:

```go
package main

import (
    "log"
    "os"

    webui "github.com/microsoft/webui/go/v3"
)

func main() {
    protocol, err := os.ReadFile("dist/protocol.bin")
    if err != nil {
        log.Fatal(err)
    }
    renderer, err := webui.NewRenderer(protocol)
    if err != nil {
        log.Fatal(err)
    }
    defer renderer.Close()

    state := []byte(`{"title":"Hello","message":"Rendered by Go and Rust"}`)
    html, err := renderer.Render(state, webui.RenderOptions{
        Entry:       "index.html",
        RequestPath: "/",
    })
    if err != nil {
        log.Fatal(err)
    }
    if _, err := os.Stdout.Write(html); err != nil {
        log.Fatal(err)
    }
}
```

`RenderOptions{}` defaults to entry `index.html` and path `/`.
`NewRenderer` enables the `webui` hydration plugin. For an HTTP application,
keep the renderer alive for the server's lifetime, pass each request's JSON
state and `request.URL.Path`, and write successful results as
`text/html; charset=utf-8`. Log rendering failures and return a generic
server error before committing a response.

The returned HTML belongs to Go and remains valid after another render or
`Close`. The protocol input may be reused after construction. Do not mutate
state bytes while `Render` is using them.

## Concurrency, closure, and errors

Concurrent `Render` calls are supported. `Close` waits for active renders,
releases resources once, and is idempotent. Explicitly close the renderer
after draining server requests; there is no finalizer. Copies share ownership,
so closing one closes every copy. Nil, zero-value, and closed renderers return
`webui.ErrClosed` from `Render`.

Rust parses JSON and returns diagnostics through `*webui.NativeError`.
Its `Operation` identifies the failed C ABI function; `Message` preserves
the native diagnostic. Use `errors.Is(err, webui.ErrClosed)` or
`errors.As` with a `*webui.NativeError` to inspect errors.

Invalid JSON and UTF-8 return errors. Embedded NUL in state bytes, entry, or
request path is rejected rather than silently truncating a C string.
JSON's escaped `\u0000` remains valid.
