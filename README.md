# WebUI

**WebUI** is a high-performance, language-agnostic server-side rendering framework built in Rust. It compiles HTML templates into compact Protocol Buffer binaries at build time so runtime rendering applies state without reparsing templates. Interactive Web Components hydrate as islands on the client.

**Documentation:** [microsoft.github.io/webui](https://microsoft.github.io/webui)

## Highlights

- **Compiled templates:** HTML is parsed once at build time into a compact binary protocol.
- **Language agnostic:** Rust, Node/Bun/Deno, C#, Python, Go, and any language that can call C FFI.
- **Web Components:** Native custom elements with Shadow DOM support.
- **Server-side logic:** Conditions, loops, and expressions are evaluated on the server.
- **Plugin-ready:** Parser and handler plugins support framework-specific hydration and directives.

## FAST plugin authored templates

The built-in FAST plugins (`fast`, `fast-v2`, and `fast-v3`) support component
HTML authored as a single wrapping `<f-template>`. A `name` attribute on that
wrapper overrides the filename-derived component tag; multiple `<f-template>`
blocks in one component source are invalid. For build-time SSR, WebUI converts
the inner FAST declarative template to WebUI syntax by rewriting `<f-repeat>` to
`<for>`, rewriting `<f-when>` to `<if>`, unwrapping directive values, and
removing client-only directives such as `@event`, `:prop`, `f-ref`,
`f-slotted`, `f-children`, and similar attributes. The authored FAST template is
preserved as the client artifact, and preserved artifacts still receive normal
template processing, normalization, and CSS injection.

## Install

```bash
npm install @microsoft/webui
```

Or install the Rust CLI:

```bash
cargo install microsoft-webui-cli
```

For .NET server-side bindings:

```bash
dotnet add package Microsoft.WebUI
```

The NuGet package restores platform-specific `Microsoft.WebUI.Runtime.*` native assets transitively. Release builds stage `.nupkg` and `.snupkg` artifacts with repository metadata and Source Link; nuget.org publishing is manual until ESRP automation supports this project.
NuGet metadata uses `Authors=Microsoft`, the `Microsoft` package owner, a stable project URL, a package license URL with license acceptance required, release notes links, discoverability tags, and the required `© Microsoft Corporation. All rights reserved.` copyright notice. Before nuget.org publishing, staged packages and Authenticode-signable contents must be signed with a Microsoft certificate through the approved signing process.

## Learn

| Resource | Link |
|----------|------|
| Full documentation | <https://microsoft.github.io/webui> |
| Getting started | <https://microsoft.github.io/webui/guide/> |
| CLI reference | <https://microsoft.github.io/webui/guide/cli/> |
| Language integrations | <https://microsoft.github.io/webui/guide/integrations> |
| Benchmarks | [`BENCHMARKS.md`](BENCHMARKS.md) |

## Development

Prerequisites:

- Rust 1.93+ with `clippy` and `rustfmt`
- Node.js 22+ with pnpm

Common commands:

| Command | Description |
|---------|-------------|
| `cargo xtask check` | Run the full repository quality gate before commits |
| `cargo xtask fmt` | Check formatting |
| `cargo xtask clippy` | Run clippy lints |
| `cargo xtask test` | Run all tests |
| `cargo xtask build` | Build the workspace and examples |
| `cargo xtask dev <app>` | Run an example app in development mode |
| `cargo xtask bench <target>` | Run benchmarks |
| `cargo xtask build-windows-local` | Manually build and stage Windows MSVC artifacts on macOS |

### Manual Windows builds on macOS

`cargo xtask build-windows-local` is a local-only helper for producing Windows
x64 and ARM64 release bits from macOS. It does not run in CI and does not
install tools automatically.

Install the build prerequisites once:

```bash
brew install llvm lld
cargo install cargo-xwin --version 0.23.0
rustup target add x86_64-pc-windows-msvc aarch64-pc-windows-msvc
```

If Homebrew's `clang-cl` and LLD are not on `PATH`, add them before running the helper:

```bash
export PATH="$(brew --prefix llvm)/bin:$(brew --prefix lld)/bin:$PATH"
```

Build both Windows targets, or choose one target:

```bash
cargo xtask build-windows-local
cargo xtask build-windows-local --target x64
cargo xtask build-windows-local --target arm64
```

The command stages artifacts into `publish/native/`, `packages/webui-win32-*`,
and `dotnet/runtimes/win-*/native/`. `cargo-xwin` downloads Microsoft Windows
SDK and CRT assets; using it requires accepting the Microsoft SDK license terms
referenced by cargo-xwin.

For a quick local sanity check of the x64 CLI artifact, install Wine Stable:

```bash
brew install --cask wine-stable
```

Wine Stable may require explicit approval in macOS **System Settings** >
**Privacy & Security** because it is not notarized. After approving it, run:

```bash
WINEDEBUG=-all WINEPREFIX="$PWD/.wine-webui-x64" \
  "/Applications/Wine Stable.app/Contents/Resources/wine/bin/wine" \
  "$PWD/packages/webui-win32-x64/bin/webui.exe" --help
```

Use Wine only for the x64 artifact. Test the ARM64 Windows artifact on Windows
ARM hardware or a Windows ARM environment.

For contribution policy, issue guidelines, and the current pull request policy, see [`CONTRIBUTING.md`](CONTRIBUTING.md).

## Project layout

```text
crates/      Rust crates for the CLI, parser, handler, protocol, FFI, and integrations
packages/    npm packages for the CLI, WebUI Framework, router, and platform binaries
dotnet/      .NET bindings, runtime packages, and global tool packaging
docs/        VitePress documentation site
examples/    Example applications and integration samples
```

## Feedback and support

WebUI is still in active development. We are not accepting unsolicited pull requests right now, but we do welcome well-documented issues:

| Need | Where to go |
|------|-------------|
| Report a bug | [Choose an issue template](https://github.com/microsoft/webui/issues/new/choose) |
| Request a feature | [Choose an issue template](https://github.com/microsoft/webui/issues/new/choose) |
| Report a documentation need | [Choose an issue template](https://github.com/microsoft/webui/issues/new/choose) |
| Get support guidance | [`SUPPORT.md`](SUPPORT.md) |
| Report a security issue | [`SECURITY.md`](SECURITY.md) |

This project has adopted the [Microsoft Open Source Code of Conduct](CODE_OF_CONDUCT.md).

## Trademarks

This project may contain trademarks or logos for projects, products, or services. Authorized use of Microsoft trademarks or logos is subject to [Microsoft's Trademark & Brand Guidelines](https://www.microsoft.com/en-us/legal/intellectualproperty/trademarks/usage/general). Use of Microsoft trademarks or logos in modified versions of this project must not cause confusion or imply Microsoft sponsorship. Any use of third-party trademarks or logos is subject to those third party policies.

## License

MIT
