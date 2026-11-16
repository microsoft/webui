# WebUI for Go

The official, zero-third-party-dependency Go binding renders compiled WebUI
templates through the native Rust C ABI. Requires Go 1.25+, cgo, a C compiler,
and a native WebUI SDK matching the module version, OS, and architecture.

```sh
go get github.com/microsoft/webui/go/v3
```

Versions are published through the repository's `go/v<version>` tags.
The root `v<version>` tags alone do not publish this nested module.
Use the matching standalone SDK archive and checksum from GitHub Releases;
npm is not required.

```go
import webui "github.com/microsoft/webui/go/v3"

protocol, err := os.ReadFile("dist/protocol.bin")
if err != nil {
    return err
}
renderer, err := webui.NewRenderer(protocol)
if err != nil {
    return err
}
defer renderer.Close()

html, err := renderer.Render(
    []byte(`{"title":"Hello","message":"Go & WebUI"}`),
    webui.RenderOptions{RequestPath: "/"},
)
if err != nil {
    return err
}
// html is Go-owned UTF-8 HTML.
```

Compile templates at build time with the version-matched CLI:

```sh
webui build ./src --out ./dist --plugin webui
```

## Native SDK setup

Supply the SDK header and library locations externally; the package does not
download an SDK, invoke a compiler at runtime, or bundle native binaries:

```sh
export CGO_ENABLED=1
export CGO_CFLAGS="-I/absolute/path/to/sdk/include"
export CGO_LDFLAGS="-L/absolute/path/to/sdk/lib"
go build ./...
```

The package links `webui_ffi` (`webui_ffi.dll.lib` on Windows). The OS loader
must also find the shared library: deploy it in the platform's library search
path, or configure `DYLD_LIBRARY_PATH` on macOS, `LD_LIBRARY_PATH` on Linux,
or `PATH` on Windows. Windows requires a cgo-compatible C toolchain.

## Ownership and errors

- Load the protocol once and reuse the renderer. The native handler uses the
  `webui` hydration plugin. No JavaScript runtime is involved.
- `RenderOptions{}` means entry `index.html` and request path `/`.
- Concurrent renders are supported. `Close` waits for active renders and is
  idempotent. Explicitly close the renderer; there is no finalizer.
- Renderer copies share ownership. Closing one closes all copies. A nil or
  zero-value renderer is closed; rendering returns `ErrClosed`.
- Protocol input is not retained. Render state must not be mutated during the
  call. Returned HTML is independent of later renders and closure.
- Rust parses JSON once and supplies native diagnostics as `*NativeError`,
  including invalid UTF-8. Embedded NUL in JSON bytes, entry, or request path is
  rejected before C-string conversion. JSON's escaped `\u0000` remains valid.
- Use `errors.Is(err, webui.ErrClosed)` or
  `errors.As(err, &nativeError)` to inspect errors.

## Tests

Tests exercise the real Rust ABI, not mocks. From the repository root, build
the source-only fixture into a temporary directory, then run from `go/`:

```sh
fixture="$(mktemp -d)"
webui build go/testdata/src --out "$fixture" --plugin webui
export WEBUI_GO_TEST_PROTOCOL="$fixture/protocol.bin"
cd go
go test -race ./...
go vet ./...
```

Keep the SDK flags and loader path configured for both commands. Missing or
unreadable `WEBUI_GO_TEST_PROTOCOL` fails actionably rather than skipping tests.
The fixture uses `index.html`, `title` and `message` state fields, and routes
`/` (`home-route`) and `/about` (`about-route`). Never commit `protocol.bin`.
