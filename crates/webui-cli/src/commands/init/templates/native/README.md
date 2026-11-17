# __APP_NAME__

An HTML-first WebUI application using the **__SERVER__** rendering host.

## Run

Install the matching WebUI native toolchain and your selected host's compiler:

```sh
webui app check
webui app dev
```

Native tooling compiles the browser TypeScript, templates and CSS, then builds
and starts your host. It does not invoke Node or npm for frontend compilation.
Node/Bun applications still require their explicitly selected server runtime
and runtime packages. Rust, Go, C, C++, and .NET do not need a JavaScript runtime.
Open <http://127.0.0.1:3000>. Stop development with Ctrl+C.

| Host | Prerequisites |
| --- | --- |
| Rust | Rust/Cargo __RUST_VERSION__ or newer |
| Go | Go 1.25+, a cgo C compiler, and the standalone native SDK |
| C / C++ | CMake 3.24+, a C/C++ toolchain, the native SDK, and the pinned CivetWeb source |
| .NET | .NET 8 SDK and an approved source for the matching Microsoft.WebUI packages |
| Node | Node.js and the installed `@microsoft/webui` runtime package |
| Bun | Bun with Node-API support and the installed `@microsoft/webui` runtime package |

Use the complete native toolchain archive for WebUI __WEBUI_VERSION__, not an
older bare CLI binary. It contains the native TypeScript checker and its
libraries. Verify its SHA-256 sidecar before extracting it. Set
`WEBUI_TOOLCHAIN_DIR` to the archive root when the compiler is not installed
beside the CLI. Nothing is acquired automatically. Frontend resources and
TypeScript paths are prepared in the ignored `.webui/` directory; application
source files are never replaced.

C, C++, and Go additionally require the standalone
`webui-sdk-__WEBUI_VERSION__-<platform>.tar.gz` archive and its checksum from
the matching release. Verify and extract its root into `.webui/sdk`, or set
`WEBUI_SDK_DIR` to its absolute directory, then run `webui sdk check`.
An invalid explicit override fails without fallback. SDKs are separate from
native compiler tooling, npm packages, and Go source modules.

The Go module is `__WEBUI_GO_MODULE__` at version __WEBUI_GO_VERSION__. Keep
the `go.sum` produced by normal Go restoration. Native hotfix identities use
the matching mapped Go version; the binding's `Version` still identifies the
exact native SDK. .NET instead uses its restored NuGet native runtime assets.

## Build and deploy

```sh
webui app build
webui app start
```

`start` uses the last complete build without compiling, restoring dependencies,
or starting a watcher. It verifies the recorded native executable and protocol.
Only `dist/public/` is served at `/assets/`; keep `dist/protocol.bin`, source,
toolchain resources, and SDKs private. Production native hosts can run directly
without the WebUI CLI, browser compiler, or selected host's development tools.
Deploy their required runtime libraries and compiled artifacts together.

Rust and Go remain ordinary root-level native projects:

```sh
webui app build --frontend-only
# Rust:
cargo check
cargo run --release
# Go, from the application root after SDK setup:
webui sdk exec -- go test ./...
webui sdk exec -- go run ./cmd/server
```

Direct Cargo/Go commands consume previously compiled frontend assets; they do
not rebuild the frontend themselves. `webui app dev` owns that orchestration.
Changes coalesce during builds; failed rebuilds wait for a source edit instead
of repeatedly retrying or serving a mismatched generation. Concurrent build,
dev, and start commands cannot publish into an actively owned application.
If promotion is interrupted, inspect `.webui/previous` before restoring or
removing generated directories.

## Edit

- `web/` contains the entry, shared layout, Home/About, and the counter island.
- `webui.toml` selects the native host and frontend paths.
- Your host's native source owns HTTP and request state.

Navigation uses ordinary document links. Both pages work without JavaScript;
the counter then remains static. About loads no executable JavaScript.
Set `HOST` and `PORT` through your environment to select the listener.
Terminate TLS at a trusted ingress.

## Authoring reference

```sh
webui reference --path
```

This prints the version-matched offline reference, with topic guides beside it.
Installing its project-local loader is a separate optional init step; no
reference installation is required to build or run the application.
