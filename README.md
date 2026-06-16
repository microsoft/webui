# WebUI

**WebUI** is a high-performance, language-agnostic server-side rendering framework built in Rust. It compiles HTML templates into compact Protocol Buffer binaries at build time so runtime rendering applies state without reparsing templates. Interactive Web Components hydrate as islands on the client.

**Documentation:** [microsoft.github.io/webui](https://microsoft.github.io/webui)

## Highlights

- **Compiled templates:** HTML is parsed once at build time into a compact binary protocol.
- **Language agnostic:** Rust, Node/Bun/Deno, C#, Python, Go, and any language that can call C FFI.
- **Web Components:** Native custom elements with Shadow DOM support.
- **Server-side logic:** Conditions, loops, and expressions are evaluated on the server.
- **Plugin-ready:** Parser and handler plugins support framework-specific hydration and directives.

## Install

```bash
npm install @microsoft/webui
```

Or install the Rust CLI:

```bash
cargo install microsoft-webui-cli
```

For the Rust library API:

```bash
cargo add microsoft-webui
```

Cargo imports the library as `webui`. The crate re-exports the core handler
API (`WebUIHandler`, `RenderOptions`, `ResponseWriter`) and built-in hydration
plugins (`FastV3HydrationPlugin`, `WebUIHydrationPlugin`) so Rust hosts can
build and render through one dependency.

For .NET server-side bindings:

```bash
dotnet add package Microsoft.WebUI
```

The NuGet package restores platform-specific `Microsoft.WebUI.Runtime.*` native assets transitively. Release builds stage `.nupkg` and `.snupkg` artifacts with repository metadata and Source Link; nuget.org publishing is manual until ESRP automation supports this project.

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
