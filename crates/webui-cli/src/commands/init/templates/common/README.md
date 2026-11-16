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
before restarting it. A failed rebuild stops the server until you fix the
source. Stop with Ctrl+C.

| Server | Additional prerequisites |
| --- | --- |
| Rust | Rust/Cargo __RUST_VERSION__ or newer |
| Node | No additional toolchain |
| Bun | Bun with Node-API support |
| .NET | .NET 8 SDK and access to the release's Microsoft.WebUI package feed |
| C / C++ | CMake 3.24+, a C/C++ toolchain, and access to the pinned CivetWeb source archive |
| Go | Go and a cgo-compatible C compiler |

The Go server imports the official `__WEBUI_GO_MODULE__` module at the
matching WebUI version. Its native ownership code is maintained in that
package, not copied into your app. Build restores the pinned Go dependency
through your configured Go proxy; keep the resulting `server/go.sum`.

C, C++, and Go use the version-matched SDK from the installed WebUI
platform package. For an offline SDK, set `WEBUI_SDK_DIR` to its root
(`sdk.json`, `include/`, `lib/`, and `LICENSE`). Build/start verify the SDK's
version, platform, and file hashes; they do not download or compile WebUI.
Keep optional npm platform dependencies enabled. If your package manager blocks
esbuild's install script, review and approve that package specifically; do not
globally enable dependency scripts.

.NET restores its native runtime assets from the matching Microsoft.WebUI
NuGet packages. Configure an approved package feed or use the official
release's packages as a local NuGet source; the npm SDK does not replace them.
For a local source, download `Microsoft.WebUI.__WEBUI_VERSION__.nupkg` and all
six `Microsoft.WebUI.Runtime.*.__WEBUI_VERSION__.nupkg` files from the
[matching release](https://github.com/microsoft/webui/releases/tag/v__WEBUI_VERSION__),
place them in one directory, and restore before building:

```sh
dotnet restore server/App.csproj --source /path/to/release-packages
```

## Edit

- `src/home-page/home-page.html` - Home content.
- `src/about-page/about-page.html` - a page with no executable JavaScript.
- `src/app-shell/` - shared layout and styles.
- `src/app-counter/` - the interactive island.
- `server/` - request state and native rendering.

Navigation uses ordinary document links. Without JavaScript, both pages work
and the counter stays disabled. On Home, hydration enables its native button.

## Build and start

```sh
__PACKAGE_MANAGER__ run check
__PACKAGE_MANAGER__ run build
__PACKAGE_MANAGER__ run start
```

`build` compiles the browser entry, templates, CSS, and selected host.
`start` uses those artifacts without compiling templates. Keep `dist/protocol.bin`
private; only `dist/public/` is served at `/assets/`. Run from the app root.
Native servers also need their built binary and runtime libraries. Node/Bun
need the installed WebUI native addon; .NET needs its runtime and restored
NuGet runtime assets.

The default listener is loopback on port 3000. Set `PORT` and `HOST` using your
deployment environment to change it. Binding a public interface is an explicit
deployment choice; terminate TLS at a trusted ingress.

## AI reference

Optionally install the existing project-local authoring reference:

```sh
npx skills add microsoft/webui --skill webui-reference
```

Its version-matched content becomes available after dependencies are installed.
See the [WebUI documentation](https://microsoft.github.io/webui/) for more.
