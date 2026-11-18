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
SDK matching `webui.Version`, your operating system, and architecture.

```bash
go get github.com/microsoft/webui/go/v3
```

Tagged Go module versions use the repository's `go/v<version>` tags,
independently of the root `v<version>` tags. A root release tag alone does not
publish the module's semantic version. Ordinary public commit pseudo-versions
are also legitimate Go module versions.

Select the native SDK using `webui.Version`, not a Go pseudo-version or
mapped hotfix version. This constant must match the exact compiler/native SDK
release identity.
Stable versions match; native `3.1.2-hotfix.1` maps to Go
`v3.1.3-0.hotfix.1` so Go orders it above `v3.1.2` and below `v3.1.3`.

Download the matching standalone `webui-sdk-<version>-<platform>.tar.gz` and
its `.sha256` file from the [release assets](https://github.com/microsoft/webui/releases).
Verify the archive against the published checksum before extracting it into
an SDK directory. The extracted root contains `include/`, `lib/`,
`sdk.json`, and license notices. npm is not required for this setup.
Checksums establish integrity, not independent authenticity; use your approved
release source.

The native packages target Windows, macOS, and GNU/Linux on x64 and ARM64.
Linux requires glibc; musl is not supported. Cross-compilation additionally
requires the target's SDK and C toolchain.

With the matching native CLI installed, extract the SDK into `.webui/sdk` and
run commands without configuring global compiler or loader paths:

```bash
webui sdk check
webui sdk exec -- go build ./...
webui sdk exec -- go test ./...
webui sdk exec -- go run .
```

`WEBUI_SDK_DIR` can select an external extracted SDK instead. It is
authoritative: invalid overrides fail without fallback. The SDK commands
validate metadata and file integrity and do not require Node/npm or acquire
SDKs automatically. See [SDK commands](/reference/cli/#webui-sdk).

Alternatively, on macOS or Linux, supply the header and library search paths:

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
state and `request.URL.EscapedPath()`, and write successful results as
`text/html; charset=utf-8`. Log rendering failures and return a generic
server error before committing a response. Pass the encoded path so WebUI can
split route segments before decoding parameters such as `a%2Fb` and `100%25`.

For nonce-based Content Security Policy, set `RenderOptions.Nonce` to a fresh,
cryptographically random nonce for each response and authorize the same value
in the response's `Content-Security-Policy` header. WebUI applies it to generated
scripts, module-preload hints, and the router's nonce metadata. An empty nonce
disables nonce emission. Nonces are request-scoped and safe to vary across
concurrent renders on the same renderer.

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

Invalid JSON and UTF-8 return errors. Embedded NUL in state bytes, entry,
request path, or nonce is rejected rather than silently truncating a C string.
JSON's escaped `\u0000` remains valid.

## Scaffold an application

[`webui init`](/reference/cli/#webui-init) creates a `net/http` application
that pins the official module for the compiler's native release identity.
Confirm the source-module version and matching SDK are available through your
approved sources. After generation, install the standalone SDK into
`.webui/sdk`, or set `WEBUI_SDK_DIR` to its root before running app commands:

```bash
webui init my-app --server go
cd my-app
webui sdk check
webui app check
webui app dev
```

Also install the matching
[native toolchain](/start/installation#native-toolchain) for browser compilation
and TypeScript checking. The default layout has root `go.mod`, host source and
tests in `cmd/server/`, frontend sources in `web/`, and `webui.toml`. There is
no `package.json` or JavaScript build orchestration.

The build restores the pinned public source module through your configured Go
proxy before compiling; commit the root `go.sum`. Use normal public module and
checksum verification, without local `replace` directives or disabling
checksums. Neither native frontend compilation nor the deployed Go renderer
needs Node/npm.

Use `webui app build`, then `webui app start` for production. Start verifies
the recorded host and protocol without compilation or restoration. The host
drains requests before closing its renderer. For direct Go commands, prepare
the frontend first and supply SDK paths to the child:

```bash
webui app build --frontend-only
webui sdk exec -- go test ./...
webui sdk exec -- go run ./cmd/server
```

Direct Go commands consume the prepared frontend rather than rebuilding it.
See [app commands](/reference/cli/#webui-app) for build/dev ownership and errors.
