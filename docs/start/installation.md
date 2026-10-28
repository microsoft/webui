---
description: Install the native WebUI CLI with npm, pnpm, Yarn, or Bun, approve package-specific setup scripts, and troubleshoot registry policies.
---

# Installation

Set up the `hello-webui` project used by the
[getting-started lessons](/start/#start-here). At the end of this page, the
local `webui` command should display its help. You will create the page itself
in the next lesson.

Use a current Node.js LTS release for the installation scripts and Node-based
tooling in the examples. Keep optional dependencies enabled: they carry the
native binary for your platform. Your package manager does not determine your
production rendering host.

## Install the CLI

For the lessons, create a directory and work inside it:

```bash
mkdir hello-webui
cd hello-webui
```

Create this file in your editor. If you are adding WebUI to an existing app,
use that app's directory and merge the scripts into its existing package file
instead.

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

Choose one package manager and keep its lockfile:

| Package manager | Install the compiler |
| --- | --- |
| npm | `npm install --save-dev @microsoft/webui` |
| pnpm | `pnpm add -D @microsoft/webui` |
| Yarn | `yarn add --dev @microsoft/webui` |
| Bun | `bun add --dev @microsoft/webui` |

**If setup scripts are blocked, complete the [package-specific approval
steps](#installation-policies) before continuing.** A downloaded package is
not necessarily a working CLI when its setup script was skipped.

Then verify the project-local command:

| Package manager | Verify the CLI |
| --- | --- |
| npm | `npm exec --no -- webui --help` |
| pnpm | `pnpm exec webui --help` |
| Yarn | `yarn run webui --help` |
| Bun | `bun run webui --help` |

The npm `--no` option refuses an implicit package installation if the local
command is missing. Do not fetch an unrelated package named `webui` to repair
an incomplete `@microsoft/webui` installation.

`@microsoft/webui` installs the native `webui` command for supported platforms.
It also exports a Node build/render API (`build`, `Protocol`) if you later
choose Node as your host. For an application that imports that API at runtime,
install it as a regular dependency rather than only a development dependency.

<span id="configure-packagejson"></span>

### Next: render a page

Continue with [Your first page](/start/first-page) in this same directory.
It creates the template and data files used by `npm run dev`. The lessons use
npm script commands; `pnpm run`, `yarn run`, and `bun run` accept the same
script names.

### Cross-Platform Support

Platform packages provide native binaries for Windows, macOS, and Linux on
x64 and ARM64. A supported prebuilt install does not need a Rust toolchain.
If the CLI is missing after installation, check for blocked setup scripts,
omitted optional dependencies, or an OS/architecture without a matching package.

Commit the package lockfile and keep compiler and browser-runtime versions
aligned. Do not substitute an unrelated globally installed CLI when diagnosing
a project-specific build problem.

### Desktop support (opt-in)

Desktop support is a separate install, not a requirement for a website:

```bash
npm install @microsoft/webui @microsoft/webui-desktop
npx webui desktop init ./my-app
npx webui desktop run ./my-app/src
```

Use matching versions. The desktop package installs a native sidecar through
its optional platform dependencies; the base CLI does not include it.
See [Desktop Apps](/build/desktop) for platform requirements, host
capabilities, and packaging.

## Installation policies

Downloading a package and permitting its install scripts are separate decisions.
`@microsoft/webui` uses a `postinstall` script to prepare the native CLI.
Review the installed package and your team's policy before approving it.
Do not enable all dependency scripts just to make one package work.

Check your manager's version with `npm --version`, `pnpm --version`,
`yarn --version`, or `bun --version`. The commands below approve the published
`@microsoft/webui` package in a local project, not a global installation.
In a workspace, use the project root that owns the script-approval policy.

### npm

**npm 12 and later** block unapproved dependency install scripts by default.
After installing, list pending scripts, approve the package you reviewed,
then run its setup:

```bash
npm install-scripts ls
npm install-scripts approve @microsoft/webui
npm rebuild @microsoft/webui
```

Approval updates `allowScripts` in `package.json`, pinned to the installed
version by default. Review the approval again after upgrading that version.
The rebuild runs the approved package's lifecycle scripts and links its command.

**npm 11 and earlier** normally run dependency scripts unless disabled; they
do not provide `npm install-scripts`. An explicit `ignore-scripts` setting or
an existing denial still needs to be resolved through your project's policy.
Do not use `--dangerously-allow-all-scripts` as the default install command.

See [npm install-script approvals](https://docs.npmjs.com/cli/v12/commands/npm-install-scripts/)
and [npm rebuild](https://docs.npmjs.com/cli/v12/commands/npm-rebuild/).

### pnpm

**pnpm 12** requires a decision for dependency build scripts. An install can
exit with `ERR_PNPM_IGNORED_BUILDS` while packages are awaiting approval.
After reviewing the package:

```bash
pnpm approve-builds @microsoft/webui
pnpm install
```

The named approval leaves other packages untouched and rebuilds this package
if it is awaiting approval. Rerunning the install completes any interrupted
installation. Decisions are recorded in `pnpm-workspace.yaml` under
`allowBuilds`, or under `permissions` when that policy is already configured
in pnpm 12.11 or later.

For **pnpm 10.1 and later versions without named approvals**, run
`pnpm approve-builds` interactively and select only the packages you reviewed.
Do not use `--all` or turn off build-script enforcement to bypass the decision.

See [pnpm approve-builds](https://pnpm.io/cli/approve-builds) and
the [pnpm 10 approval workflow](https://pnpm.io/10.x/cli/approve-builds).

### Yarn

**Modern Yarn (2 and later)** controls third-party setup scripts through
`enableScripts` and `dependenciesMeta`. If scripts are disabled, merge this
package-specific approval into the root `package.json`, preserving any existing
entries:

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

This allows the reviewed package without setting `enableScripts: true` for
every dependency. Do not override an intentional package denial without
reviewing it with the project owner.

**Yarn Classic (1.x)** normally runs dependency scripts unless disabled with
settings such as `ignore-scripts`. It does not use this modern Yarn approval
and rebuild workflow.

See [Yarn script policy](https://yarnpkg.com/configuration/yarnrc#enableScripts),
[per-package build permissions](https://yarnpkg.com/configuration/manifest#dependenciesMeta.built),
and [yarn rebuild](https://yarnpkg.com/cli/rebuild).

### Bun

Bun runs dependency lifecycle scripts only for trusted packages. If it reports
that WebUI's script was blocked, inspect pending packages and trust only the
package you reviewed:

```bash
bun pm untrusted
bun pm trust @microsoft/webui
```

The trust command runs its pending scripts and records the package in
`trustedDependencies` in `package.json`. Review the resulting list: an explicit
`trustedDependencies` list replaces Bun's built-in default list rather than
extending it. Preserve other reviewed entries; do not use `bun pm trust --all`.

See [Bun lifecycle scripts](https://bun.sh/docs/pm/lifecycle) and
[bun pm trust](https://bun.sh/docs/pm/cli/pm#trust).

### Other tools and CI

Bundlers such as `esbuild` may also need script approval. Review each reported
package separately and apply the matching manager's approval process; approving
`@microsoft/webui` does not approve every dependency.

Commit the resulting project approval settings and lockfile so teammates and
CI use the reviewed policy. An explicit script-disable setting can still
prevent setup; resolve that policy deliberately instead of adding a global
override.

### Download restrictions

Script approval does not fix a rejected download, authentication failure, or
proxy error.

In **npm 12**, `allow-remote` defaults to `none`, but tarballs on the configured
registry's hostname are still allowed. An ordinary install from that registry
does not require `--allow-remote=all`. Direct tarball URL dependencies and
registries that serve tarballs from a different hostname can require additional
configuration.

If npm reports a remote-URL rejection, check the exact error, the configured
registry (including any `@microsoft` scope override), and the tarball hostname.
Use your organization's supported registry configuration or ask its registry
administrator to review the mismatch. Do not add `--allow-remote=all` to every
install command. See npm's [allow-remote](https://docs.npmjs.com/cli/v12/using-npm/config/#allow-remote)
and [replace-registry-host](https://docs.npmjs.com/cli/v12/using-npm/config/#replace-registry-host)
settings.

The npm flag is not a shared option for pnpm, Yarn, or Bun. Those managers have
their own registry, proxy, offline, and trust settings. Diagnose the reported
download error separately from script approvals. When requesting help, include
the manager version, sanitized error, and registry/tarball hostnames, not tokens
or an unredacted configuration file.

## Rust

Building from source requires Rust 1.99.1 or newer.
Rust users can install the same compiler through Cargo:

```bash
cargo install microsoft-webui-cli
webui build ./src --out ./dist
```

This is an alternative to the npm compiler install, not another first-app step.
The [Rust integration guide](/build/integrations/rust) covers rendering in a
production Rust service. See the [CLI reference](/reference/cli/) for command options.

## .NET

The managed binding is `Microsoft.WebUI`. It loads compiled protocols and
uses platform-specific `Microsoft.WebUI.Runtime.*` native assets.

**Check package availability before choosing an install command.** The release
pipeline stages NuGet artifacts, but automatic NuGet.org publishing is not
enabled pending an approved signing path. Obtain the packages from an available
release or approved feed; do not assume a public restore will succeed.

The [.NET integration guide](/build/integrations/dotnet) covers supported
targets, protocol lifetime, and ASP.NET response handling.

## Python

The native `microsoft-webui` binding is **not published to PyPI yet**. Install
a matching wheel from a GitHub Release, or build from source with `maturin`:

```bash
pip install ./microsoft_webui-<version>-cp311-abi3-<platform>.whl
```

Replace the placeholder with the wheel you downloaded for your platform.
The binding supports CPython 3.11+ and renders protocols; it does not compile
templates. Build `protocol.bin` with the npm or Cargo CLI first.
See [Python integration](/build/integrations/python) for installation details
and WSGI/ASGI examples.

## WebUI Framework (Client-Side Interactivity)

Install the browser runtime only when the app needs browser behavior or
browser-applied state:

```bash
npm install @microsoft/webui-framework
```

It supplies `WebUIElement`, `@attr`, and `@observable`, and connects interactive
components to server-rendered HTML. It is separate from the compiler and server
binding.

A component that only renders data can remain HTML and CSS. Do not add an
empty TypeScript class just to display bindings. When you do add one, configure
the decorators and browser bundler as shown in
[Add interaction](/start/add-interactivity#bundle-the-browser-entry).

## Client-Side Router (Optional)

Ordinary document navigation needs no client router. For soft navigation:

```bash
npm install @microsoft/webui-delivery
```

The router is a separate browser package, with server-side route and partial
response requirements. It works with WebUI Framework and FAST 3.x components.
Follow [Routing](/build/routing) when the application needs it, not as
a prerequisite to the counter tutorial.

## AI Coding Agents

`@microsoft/webui` includes the version-matched `ai.md` authoring reference.
The `webui-reference` [agent skill](https://agentskills.io) tells supported
coding agents to read that file from the application's installed dependencies.

After installing `@microsoft/webui`, install the loader:

```bash
npx skills add microsoft/webui --skill webui-reference
```

For pnpm, use `pnpm dlx skills add microsoft/webui --skill webui-reference`;
for modern Yarn, use `yarn dlx skills add microsoft/webui --skill webui-reference`;
for Bun, use `bunx skills add microsoft/webui --skill webui-reference`.

The loader lives in `.agents/skills/webui-reference/`, with agent-specific
installation locations where needed. Commit a project-local installation if
you want to share it with your team.

| Option | Purpose |
| --- | --- |
| `-g` | Install in the agent's user scope rather than this project |
| `-a <agent>` | Target an agent, for example `-a github-copilot` |
| `-l` | List available skills without installing them |

Upgrading `@microsoft/webui` updates its reference; ask an active agent to reread
it. Rerun the install command only when you want to update the loader itself.
If you previously installed the full reference as a skill, reinstall in the
same scope to replace it with the loader.

The loader resolves the package from the target app, including hoisted
dependencies, even when the skill is global. In the WebUI source checkout it
reads `docs/ai.md` instead. If a matching installed reference is missing, it
reports that rather than silently using a newer release's rules.

To try the loader for one Copilot session:

```bash
npx skills use microsoft/webui@webui-reference | copilot
```

You can also read the installed file yourself. The website's
[AI Reference](/ai) follows the current release and may be newer than your
application.
