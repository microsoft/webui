---
description: Install the WebUI CLI, verify your setup, and choose a backend integration when you are ready to deploy.
---

# Installation

Choose native application scaffolding, or set up the `hello-webui` project for
the [getting-started lessons](/start/). Native frontend tooling does not need
Node.js; the package-managed lesson setup below uses a current Node.js LTS
release. Your backend can use any of the [deep integrations](#deep-integrations).

## Scaffold an application

For a complete small application instead of the step-by-step lessons, use
the WebUI initializer with a [native toolchain](#native-toolchain) and your
selected server's prerequisites installed:

```bash
webui init my-app --server rust --no-interactive
cd my-app
webui app check
webui app dev
```

Rust is the default server. Other choices are `dotnet`, `c`, `cpp`, `go`,
`node`, and `bun`; install the
selected host's toolchain separately. Native frontend tooling is used
for every server choice. Rust, Go, C, C++, and .NET starters have no
`package.json` or JavaScript build scripts. Node/Bun starters retain their
runtime `package.json` and server code; restore those packages with `npm install`
or `bun install`, respectively. The starter includes server-rendered
Home and About pages, native links, and one interactive counter. About needs
no JavaScript. No UI kit, database, authentication, or client router is added.

File generation is offline once the CLI is installed. Init never overwrites
existing application files. In a terminal it offers dependency installation
and installation of the [existing AI reference skill](#ai-coding-agents);
both are optional. `--install` restores the selected host's dependencies; it
does not install system toolchains or acquire a standalone SDK. For native
projects, `--ai-skill` installs the project-local loader offline.

Rust starts with `Cargo.toml`, `src/main.rs`, and `web/`; Go starts with
`go.mod`, `cmd/server/`, and `web/`. All native starters use `webui.toml`.
Use `webui app build` for production output, then `webui app start` to run
the recorded build without restoration or compilation.

See [`webui init`](/reference/cli/#webui-init) for automation flags, package
manager selection, and machine-readable output. To follow the lessons instead,
continue with the manual setup below.

## Native toolchain

Native application checking requires the CLI and its matching native TypeScript
compiler. Obtain a complete
`webui-toolchain-<version>-<target-triple>.tar.gz` and adjacent SHA-256 checksum
from an approved distribution that provides that artifact. Check availability
for your selected release and target before choosing this setup; a bare CLI
or an npm binary package is not the complete native checker toolchain.

Verify the archive before extraction. Keep its layout intact:

```text
webui-toolchain/
  bin/webui                 # webui.exe on Windows
  compiler/tsc              # tsc.exe on Windows; lib*.d.ts beside it
  notices/
  toolchain.json
```

Checksums establish file integrity, not independent authenticity; use your
approved artifact source. Consumers need this toolchain and their chosen
server's prerequisites, not the tools used to produce the archive.

Add the extracted `bin/` to your command path. Alternatively set
`WEBUI_TOOLCHAIN_DIR` to the extracted root while using the matching CLI.
An explicit override is authoritative, including when invalid. Native checking
validates the exact release, platform, file inventory, and hashes before
running the compiler. TypeScript is pinned to 7.0.2, independently of WebUI's
release version. It never uses a PATH `tsc`, JavaScript wrapper, npm lookup,
or automatic download. Supported targets are macOS, GNU/Linux, and MSVC Windows
on x64 and ARM64; musl is not supported.

Install the selected server prerequisites separately:

| Server | Prerequisites |
| --- | --- |
| Rust | Rust 1.100.0+ and Cargo |
| Go | Go 1.25+, cgo C compiler, matching [standalone SDK](/build/integrations/go#installation) |
| C/C++ | CMake 3.24+, native C/C++ toolchain, matching [standalone SDK](/build/integrations/ffi#standalone-native-sdk) |
| .NET | .NET 8 SDK and an approved matching [Microsoft.WebUI package source](/build/integrations/dotnet) |
| Node | Node.js 22+ with native-addon support |
| Bun | Bun with Node-API support |

Toolchain and C ABI SDK acquisition are separate. Go/C/C++ resolve their SDK
from `.webui/sdk` or the explicit `WEBUI_SDK_DIR`; `webui sdk check` verifies
it. Rust uses Cargo, .NET uses NuGet runtime packages, and Node/Bun use the
native addon instead.

For standalone template compilation, the CLI can also be built without Node:

```bash
cargo install microsoft-webui-cli
```

This needs Rust 1.100.0+ and does not by itself install the native TypeScript
companion required by `webui app check`.

<span id="install-the-cli"></span>

## Package-managed lesson setup

Create a project directory:

```bash
mkdir hello-webui
cd hello-webui
```

Create this file, or merge the scripts into your existing package file:

**`package.json`**

```json
{
  "name": "hello-webui",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "webui serve ./src --state ./data/state.json",
    "build": "webui build ./src --out ./dist"
  }
}
```

Choose a package manager. Your browser remembers the selection for the homepage
installer and package-manager tabs across these guides:

<webui-press-tabs sync="package-manager" storage="local">
<webui-press-tab slot="tab" value="npm" active>npm</webui-press-tab>
<webui-press-tab slot="tab" value="pnpm">pnpm</webui-press-tab>
<webui-press-tab slot="tab" value="yarn">Yarn</webui-press-tab>
<webui-press-tab slot="tab" value="bun">Bun</webui-press-tab>
<webui-press-tab-panel active>

```bash
npm install --save-dev @microsoft/webui
```

</webui-press-tab-panel>
<webui-press-tab-panel>

```bash
pnpm add -D @microsoft/webui
```

</webui-press-tab-panel>
<webui-press-tab-panel>

```bash
yarn add --dev @microsoft/webui
```

</webui-press-tab-panel>
<webui-press-tab-panel>

```bash
bun add --dev @microsoft/webui
```

</webui-press-tab-panel>
</webui-press-tabs>

Keep optional dependencies enabled: they provide native binaries for Windows,
macOS, and Linux on x64 and ARM64. A supported prebuilt install needs no Rust
toolchain. Commit your lockfile and keep WebUI package versions aligned.

The same package installs the `webui` CLI and ships separate native runtime
and compiler addons. Node hosts import runtime APIs from `@microsoft/webui`;
custom Node build scripts import compiler APIs from `@microsoft/webui/build.js`.
Each API loads only its own addon. See the [Node integration guide](/build/integrations/node#choose-an-entry-point).

## Verify the CLI

<webui-press-tabs sync="package-manager" storage="local">
<webui-press-tab slot="tab" value="npm" active>npm</webui-press-tab>
<webui-press-tab slot="tab" value="pnpm">pnpm</webui-press-tab>
<webui-press-tab slot="tab" value="yarn">Yarn</webui-press-tab>
<webui-press-tab slot="tab" value="bun">Bun</webui-press-tab>
<webui-press-tab-panel active>

```bash
npm exec --no -- webui --help
```

</webui-press-tab-panel>
<webui-press-tab-panel>

```bash
pnpm exec webui --help
```

</webui-press-tab-panel>
<webui-press-tab-panel>

```bash
yarn run webui --help
```

</webui-press-tab-panel>
<webui-press-tab-panel>

```bash
bun run webui --help
```

</webui-press-tab-panel>
</webui-press-tabs>

The command should display its help. npm's `--no` refuses an implicit package
installation if the local command is missing. Do not install an unrelated
package named `webui` or a global CLI to repair this project.

<span id="configure-packagejson"></span>

## Create your first page

Continue with [Your first page](/start/first-page) in this directory. It creates
the HTML template and JSON data used by the `dev` script. No browser runtime
or production server integration is needed for that lesson.

## Installation policies

<details>
<summary>Setup scripts blocked?</summary>

Downloading a package and permitting its install scripts are separate decisions.
`@microsoft/webui` uses `postinstall` to prepare the native CLI. Review the
package and your team's policy before approving it. In a workspace, use the
root that owns the script-approval policy.

<webui-press-tabs sync="package-manager" storage="local">
<webui-press-tab slot="tab" value="npm" active>npm</webui-press-tab>
<webui-press-tab slot="tab" value="pnpm">pnpm</webui-press-tab>
<webui-press-tab slot="tab" value="yarn">Yarn</webui-press-tab>
<webui-press-tab slot="tab" value="bun">Bun</webui-press-tab>
<webui-press-tab-panel active>

**npm 12 and later** block unapproved dependency install scripts by default.
List pending scripts, approve the package you reviewed, then run its setup:

```bash
npm install-scripts ls
npm install-scripts approve @microsoft/webui
npm rebuild @microsoft/webui
```

Approval updates `allowScripts` in `package.json`, pinned to the installed
version by default. Review it again after upgrading. The rebuild runs the
approved lifecycle scripts and links the command.

**npm 11 and earlier** normally run dependency scripts unless disabled and
do not provide `npm install-scripts`. Resolve an explicit `ignore-scripts`
setting or denial through your project's policy. Do not use
`--dangerously-allow-all-scripts` as the default install command.

See [npm install-script approvals](https://docs.npmjs.com/cli/v12/commands/npm-install-scripts/)
and [npm rebuild](https://docs.npmjs.com/cli/v12/commands/npm-rebuild/).

</webui-press-tab-panel>
<webui-press-tab-panel>

**pnpm 12** requires a decision for dependency build scripts. An install can
exit with `ERR_PNPM_IGNORED_BUILDS` while packages await approval.
After reviewing the package:

```bash
pnpm approve-builds @microsoft/webui
pnpm install
```

Named approval leaves other packages untouched and rebuilds this package if
it is awaiting approval. Rerunning the install completes an interrupted setup.
Decisions are recorded in `pnpm-workspace.yaml` under `allowBuilds`, or under
`permissions` when that policy is configured in pnpm 12.11 or later.

For **pnpm 10.1 and later versions without named approvals**, run
`pnpm approve-builds` interactively and select only reviewed packages.
Do not use `--all` or disable build-script enforcement.

See [pnpm approve-builds](https://pnpm.io/cli/approve-builds) and
the [pnpm 10 approval workflow](https://pnpm.io/10.x/cli/approve-builds).

</webui-press-tab-panel>
<webui-press-tab-panel>

**Modern Yarn (2 and later)** controls setup scripts through `enableScripts`
and `dependenciesMeta`. If scripts are disabled, merge this approval into the
root `package.json`, preserving existing entries:

```json
{
  "dependenciesMeta": {
    "@microsoft/webui": {
      "built": true
    }
  }
}
```

Then run:

```bash
yarn install
yarn rebuild @microsoft/webui
```

This allows the reviewed package without enabling scripts for every dependency.
Do not override an intentional denial without reviewing it with the project owner.

**Yarn Classic (1.x)** normally runs dependency scripts unless disabled.
It does not use this modern Yarn approval and rebuild workflow.

See [Yarn script policy](https://yarnpkg.com/configuration/yarnrc#enableScripts),
[per-package build permissions](https://yarnpkg.com/configuration/manifest#dependenciesMeta.built),
and [yarn rebuild](https://yarnpkg.com/cli/rebuild).

</webui-press-tab-panel>
<webui-press-tab-panel>

Bun runs lifecycle scripts only for trusted packages. Inspect pending packages
and trust only the package you reviewed:

```bash
bun pm untrusted
bun pm trust @microsoft/webui
```

The trust command runs pending scripts and records the package in
`trustedDependencies` in `package.json`. An explicit list replaces Bun's
built-in default list, so preserve other reviewed entries.
Do not use `bun pm trust --all`.

See [Bun lifecycle scripts](https://bun.sh/docs/pm/lifecycle) and
[bun pm trust](https://bun.sh/docs/pm/cli/pm#trust).

</webui-press-tab-panel>
</webui-press-tabs>

Other development tools may need separate approval. Commit the reviewed
settings and lockfile for teammates and CI. Do not enable all dependency
scripts to make one package work.

</details>

<details>
<summary>Download, registry, or proxy errors?</summary>

Script approval does not fix a rejected download or authentication failure.
Check the exact error, manager version, configured registry, and tarball
hostname. Follow your organization's registry and proxy policy.

In **npm 12**, `allow-remote` defaults to `none`, but tarballs on the configured
registry's hostname are allowed. Direct URL dependencies or registries serving
tarballs from another hostname can need additional configuration.
Do not add `--allow-remote=all` to every install command.
See [allow-remote](https://docs.npmjs.com/cli/v12/using-npm/config/#allow-remote)
and [replace-registry-host](https://docs.npmjs.com/cli/v12/using-npm/config/#replace-registry-host).
This npm flag is not shared by pnpm, Yarn, or Bun.

When requesting help, share a sanitized error and hostnames, not tokens or
an unredacted configuration file.

</details>

## Deep integrations

Choose the backend that fits your application. These integrations all use
the native renderer; the package manager used above does not choose your host.
Each guide covers installation, protocol lifetime, and server integration.

| Backend | Guide |
| --- | --- |
| Rust | [Build and render with the Rust API](/build/integrations/rust) |
| Node.js and Bun | [Use the native addon](/build/integrations/node) |
| .NET | [Integrate with ASP.NET](/build/integrations/dotnet) |
| Python | [Integrate with WSGI or ASGI](/build/integrations/python) |
| Go | [Use the native Go binding](/build/integrations/go) |
| C and other languages | [Use the C ABI](/build/integrations/ffi) |
| WebAssembly | [Render in a browser host](/build/integrations/wasm) |

## Add what your app needs

- [Browser interaction](/start/add-interactivity) adds the optional component runtime.
- [Routing](/build/routing) adds client-side navigation.
- [Desktop apps](/build/desktop) adds a native system-webview host.
- [AI authoring reference](/ai) explains the version-matched reference and agent skill.

## AI coding agents

For a native application, install the offline project-local loader while
creating it:

```bash
webui init my-app --ai-skill --no-interactive
```

The loader uses the version-matched native CLI's `webui reference --path`
to locate the core reference and adjacent Markdown guides. It does not need
Node/npm or a network. Keep the CLI version aligned with `webui.toml`.

For a Node-frontend application, install the reference skill in your project:

```bash
npx skills add microsoft/webui --skill webui-reference
```

Add `--global` to install the loader for your user account across projects.
In a Node-frontend application, the skill reads `@microsoft/webui/ai.md` and relevant
Markdown guides bundled with that installed package, including in hoisted
workspaces. These guides work offline and match the package release. Install
`@microsoft/webui` even if you only imported the browser framework, and keep
their versions aligned.

Upgrading the package updates its reference without reinstalling the skill.
If you installed an older loader before local guide support, reinstall the
skill once in the same scope. If a guide is missing, the agent should report it
rather than substitute the latest online documentation. The website's
interactive tools and external links still require network access.
