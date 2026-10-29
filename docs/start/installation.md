---
description: Install the WebUI CLI, verify your setup, and choose a backend integration when you are ready to deploy.
---

# Installation

Set up the `hello-webui` project for the [getting-started lessons](/start/).
You need a current Node.js LTS release for the installation scripts and tooling.
Your production backend can use any of the [deep integrations](#deep-integrations).

## Install the CLI

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

Bundlers such as `esbuild` may need separate approval. Commit the reviewed
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
| C and other languages | [Use the C ABI](/build/integrations/ffi) |
| WebAssembly | [Render in a browser host](/build/integrations/wasm) |

## Add what your app needs

<span id="ai-coding-agents"></span>

- [Browser interaction](/start/add-interactivity) adds the optional component runtime.
- [Routing](/build/routing) adds client-side navigation.
- [Desktop apps](/build/desktop) adds a native system-webview host.
- [AI authoring reference](/ai) explains the version-matched reference and agent skill.
