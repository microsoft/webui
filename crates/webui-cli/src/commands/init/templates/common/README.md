# __APP_NAME__

A two-page WebUI app using the **__SERVER__** server.

## Run

Install Node.js 22 or newer and your selected server's toolchain, then run
these commands from this directory:

```sh
__PACKAGE_MANAGER__ install
__PACKAGE_MANAGER__ run check
__PACKAGE_MANAGER__ run dev
```

Open <http://127.0.0.1:3000>. Development watches your source files and runs the
selected server. Changes rebuild the matching browser assets and protocol
before restarting it. Native filesystem events watch source directories and
configuration files; generated output is ignored. Edits during a build
coalesce into a follow-up build. A failed rebuild stops the server until you fix the
source. Stop with Ctrl+C.

| Server | Additional prerequisites |
| --- | --- |
| Rust | Rust/Cargo __RUST_VERSION__ or newer |
| Node | No additional toolchain |
| Bun | Bun with Node-API support |
| .NET | .NET 8 SDK and an approved source for the matching Microsoft.WebUI packages |
| C / C++ | CMake 3.24+, a C/C++ toolchain, and access to the pinned CivetWeb source archive |
| Go | Go 1.25+ and a cgo-compatible C compiler |

C, C++, and Go require the standalone WebUI __WEBUI_VERSION__ native SDK.
Download the `webui-sdk-__WEBUI_VERSION__-<platform>.tar.gz` archive and its
adjacent `.sha256` file from the
[matching release](https://github.com/microsoft/webui/releases/tag/v__WEBUI_VERSION__).
Verify the checksum and extract the SDK root (`sdk.json`, `include/`, `lib/`,
and `LICENSE`) into `.webui/sdk`, or set `WEBUI_SDK_DIR` to its absolute path.
Build/start validate the version, target, OS, architecture, and file hashes.
An invalid explicit override fails without fallback. SDKs are never taken
from npm packages or `node_modules`, downloaded, or built automatically.

The Go host imports `__WEBUI_GO_MODULE__` at module version __WEBUI_GO_VERSION__;
it does not copy a native binding into your app. Build restores the dependency
through your configured Go proxy; keep the resulting `server/go.sum`.
Stable module and native versions match. Hotfix module versions sort above
their base release; `webui.Version` still identifies the exact native SDK
(__WEBUI_VERSION__), not the mapped module version.

.NET uses restored NuGet native runtime assets, not the standalone SDK.
Configure an approved source or download `Microsoft.WebUI.__WEBUI_VERSION__.nupkg`
and all six `Microsoft.WebUI.Runtime.*.__WEBUI_VERSION__.nupkg` files from the
matching release into one directory. Restore from that local source before
building, without changing global NuGet configuration:

```sh
dotnet restore server/App.csproj --source /path/to/release-packages
```

Keep optional npm platform dependencies enabled. If your package manager blocks
esbuild's install script, review and approve that package specifically; do not
globally enable dependency scripts.
Use your selected package manager's `run` commands. Yarn starters include a
project-local `.yarnrc.yml` selecting the `node-modules` linker for the native
TypeScript compiler; no global Yarn settings are changed.

## Edit

- `src/home-page/home-page.html` - Home content.
- `src/about-page/about-page.html` - a page with no executable JavaScript.
- `src/app-shell/` - shared layout and styles.
- `src/app-counter/` - the interactive island.
- `server/` - request state and native rendering.

Navigation uses ordinary document links. Without JavaScript, both pages work
and the counter remains static. On Home, hydration wires its native button.

## Build and start

```sh
__PACKAGE_MANAGER__ run check
__PACKAGE_MANAGER__ run build
__PACKAGE_MANAGER__ run start
```

`build` compiles the browser entry, templates, CSS, and selected host.
`start` uses those artifacts without compiling templates. Keep `dist/protocol.bin`
private; only `dist/public/` is served at `/assets/`. Run from the app root.
Native servers need their built binary and runtime libraries. Node/Bun need
the installed WebUI native addon; .NET needs its runtime and restored NuGet
runtime assets. C/C++/Go render without a JavaScript runtime. Node remains
build tooling for browser JavaScript, not a deployed native server dependency.
Rust builds explicitly use the app-owned `server/target/` directory, even when
`CARGO_TARGET_DIR` is set. Cargo's reported executable is recorded in
`.webui/server-executable.json`, so build, dev, and start share the same binary
even with a configured Cargo target. Keep that metadata when deploying the Rust
app; start does not require Cargo.

The default listener is loopback on port 3000. Set `PORT` and `HOST` using your
deployment environment to change it. Binding a public interface is an explicit
deployment choice; terminate TLS at a trusted ingress.

C/C++ reuse HTTP/1.1 connections for bodyless requests, including HEAD and
error responses. These read-only hosts close body-bearing requests after
responding instead of consuming uploads. Their transport allows at most 16
on-demand workers, 16 queued connections, 100 ms between persistent requests,
and 2 seconds for request I/O. Adjust `server/host.h` for your deployment's
concurrency and slow-client requirements.

## AI reference

Optionally install the existing project-local authoring reference:

```sh
npx skills add microsoft/webui --skill webui-reference
```

Its version-matched content becomes available after dependencies are installed.
See the [WebUI documentation](https://microsoft.github.io/webui/) for more.
