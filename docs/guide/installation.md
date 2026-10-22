---
description: Install the native WebUI CLI from npm, then add browser behavior or a production host only when needed.
---

# Installation

For a new app, use the **npm CLI** and follow
[Hello World](/tutorials/hello-world). It renders a first page without Rust,
a source checkout, or a browser runtime.

Use a current Node.js LTS release and keep npm's optional dependencies enabled.
Node installs the tools and runs your bundler; it does not have to be your
production rendering host.

## npm

From your application's directory:

```bash
npm init -y
npm install --save-dev @microsoft/webui
npx webui --help
```

`@microsoft/webui` installs the native `webui` command for supported platforms.
It also exports a Node build/render API (`build`, `Protocol`) if you later
choose Node as your host. For an application that imports that API at runtime,
install it as a regular dependency rather than only a development dependency.

Prefer pnpm or Yarn? Use `pnpm add -D @microsoft/webui` or
`yarn add -D @microsoft/webui`, then run the local CLI through your package
manager. There is no `create-webui` scaffolder required by this guide.

### Configure package.json

These scripts assume the files created in
[Hello World](/tutorials/hello-world#project-setup):

```json
{
  "scripts": {
    "dev": "webui serve ./src --state ./data/state.json",
    "build": "webui build ./src --out ./dist"
  }
}
```

Merge the `scripts` object into your existing `package.json`; do not remove
the dependencies npm added. Run from the app root:

```bash
npm run dev
npm run build
```

`dev` previews at **http://127.0.0.1:3000/** until you stop it with Ctrl+C.
`build` emits `dist/protocol.bin`. That artifact needs a renderer; it is not a
standalone HTML site.

For interactive components you will also bundle browser TypeScript and serve
the resulting assets. `webui build` does **not** perform that bundling step.
Use the [complete counter setup](/tutorials/hello-world#adding-interactivity)
rather than adding only a script tag.

### Cross-Platform Support

Platform packages provide native binaries for Windows, macOS, and Linux on
x64 and ARM64. A supported prebuilt install does not need a Rust toolchain.
If the CLI is missing after installation, check whether optional dependencies
were omitted or your OS/architecture lacks a matching package.

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
See [Desktop Apps](/guide/concepts/desktop) for platform requirements, host
capabilities, and packaging.

## Rust

Rust users can install the same compiler through Cargo:

```bash
cargo install microsoft-webui-cli
webui build ./src --out ./dist
```

This is an alternative to the npm compiler install, not another first-app step.
The [Rust integration guide](/guide/integrations/rust) covers rendering in a
production Rust service. See the [CLI reference](/guide/cli/) for command options.

## .NET

The managed binding is `Microsoft.WebUI`. It loads compiled protocols and
uses platform-specific `Microsoft.WebUI.Runtime.*` native assets.

**Check package availability before choosing an install command.** The release
pipeline stages NuGet artifacts, but automatic NuGet.org publishing is not
enabled pending an approved signing path. Obtain the packages from an available
release or approved feed; do not assume a public restore will succeed.

The [.NET integration guide](/guide/integrations/dotnet) covers supported
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
See [Python integration](/guide/integrations/python) for installation details
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
[Hello World](/tutorials/hello-world#bundle-the-browser-entry).

## Client-Side Router (Optional)

Ordinary document navigation needs no client router. For soft navigation:

```bash
npm install @microsoft/webui-router
```

The router is a separate browser package, with server-side route and partial
response requirements. It works with WebUI Framework and FAST 3.x components.
Follow [Routing](/guide/concepts/routing) when the application needs it, not as
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
for Yarn, use `yarn dlx skills add microsoft/webui --skill webui-reference`.

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
