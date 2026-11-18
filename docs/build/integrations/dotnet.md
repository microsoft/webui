---
title: ".NET"
description: "Use Microsoft.WebUI with matching native runtime packages, safe protocol ownership, and ASP.NET streaming responses."
---

# .NET

The `Microsoft.WebUI` NuGet package wraps the Rust renderer's C ABI with safe
handles and managed result types. It targets .NET 8 and .NET 9 and restores
matching native runtime packages. Load one `Protocol` for repeated renders,
then dispose native-backed objects when their work is complete.

## Installation

**Check package availability first.** NuGet artifacts are staged by the release
pipeline, but automatic NuGet.org publishing is not enabled pending an approved
signing path. Obtain matching packages from an available release or approved
feed; the command below requires that feed to be configured.

[`webui init`](/reference/cli/#webui-init) with `--server dotnet` creates
an ASP.NET Core app pinned to the compiler's WebUI version. Configure an
approved source for that version. The native frontend default creates root
`App.csproj`, `Program.cs`, `web/`, and `webui.toml`, without a `package.json`
or JavaScript build scripts. Install the
[native toolchain](/start/installation#native-toolchain) for browser compilation
and native TypeScript checking; Node/npm are not required.

Alternatively, download the managed
`Microsoft.WebUI` `.nupkg` and all six `Microsoft.WebUI.Runtime.*` `.nupkg`
files from the same [official release](https://github.com/microsoft/webui/releases)
into one directory, then restore from the app root:

```bash
dotnet restore App.csproj --source /path/to/release-packages
```

The generated build uses restored NuGet native runtime assets, not npm or
`WEBUI_SDK_DIR`. It does not configure global feeds, fall back to an older
package, or change package publication policy.

```bash
webui app check
webui app dev
webui app build
webui app start
```

The .NET host check performs a native build, so it cannot run while another app
build/dev/start owns the project. Start verifies the last complete host/protocol
build without compiling or restoring. The generated server drains requests
before disposing native render objects. For direct `dotnet` builds, first run
`webui app build --frontend-only` to prepare browser assets and the protocol.
Only `dist/public/` is public static content.

See
[app commands](/reference/cli/#webui-app).

```bash
dotnet add package Microsoft.WebUI
```

Load the protocol once and reuse it:

```csharp
using Microsoft.WebUI;

using var protocol = new Protocol(
    await File.ReadAllBytesAsync("dist/protocol.bin"));
using var handler = new WebUIHandler("webui");

string html = handler.Render(
    protocol,
    """{"title":"Home"}""",
    "index.html",
    "/");
```

## Lazy template packs

[Lazy bodies](/learn/lazy) need no .NET API. `webui build` writes their
template packs to `<out>/_webui/packs/`; serve that directory as immutable
static files, for example with `UseStaticFiles`, and render pages with the same
`Protocol`.

## Progressive streaming

`StreamResponse` creates a single-driver session. `Start`, `Resume`, and
`Advance` return a `StreamingStep` containing `Bytes`, `Done`, and an optional
`Boundary` descriptor.

```csharp
using var session = handler.StreamResponse(protocol, "index.html", "/");

Response.ContentType = "text/html; charset=utf-8";
StreamingStep step = session.Start(initialStateJson);

while (true)
{
    await Response.Body.WriteAsync(step.Bytes);
    await Response.Body.FlushAsync();
    if (step.Done) break;

    if (step.Boundary is BoundaryDescriptor boundary)
    {
        string state = await LoadBoundaryStateAsync(
            boundary.Owner,
            boundary.Name,
            boundary.Key);
        step = session.Resume(
            boundary.InstanceId,
            state,
            BoundaryMode.Final);
    }
    else
    {
        step = session.Advance();
    }
}
```

| Member | Result |
|---|---|
| `Start(stateJson)` | Shell bytes through the first descriptor or terminal |
| `Resume(instanceId, stateJson, mode)` | Only the pending occurrence's bytes through its checkpoint |
| `Advance()` | Following parent bytes through the next descriptor or terminal |
| `Update(instanceId, patchJson)` | Projected state bytes for an updatable occurrence |

A descriptor requires `Resume`; no descriptor with `Done == false` requires
`Advance`; `Done == true` means complete. Flush the boundary-only `Resume`
result before waiting for more data. `Advance` renders the following parent or
tail bytes.

The descriptor contains:

- `InstanceId`, unique within this response
- `DeclarationId`, stable within the compiled protocol
- `Owner`, the entry or component template that authored the declaration
- `Name`, local to that owner
- `Key`, a `BoundaryKey` with `Type`, `StringValue`, and `NumberValue`

Commit an occurrence as `BoundaryMode.Updatable` to send later state:

```csharp
byte[] chunk = session.Update(
    searchInstanceId,
    """{"query":"webui"}""");
await Response.Body.WriteAsync(chunk);
await Response.Body.FlushAsync();
```

Updates apply projected state to existing roots. They do not insert markup or
rerun hydration, and are valid between an occurrence's `Resume` and `Advance`.
The step that reports `Done` already includes the response tail and terminal
record.

Drive one session from one request flow at a time. Independent sessions may run
concurrently against the same handler and protocol. `WebUIException` carries
the native diagnostic for invalid state, ordering, keys, or rendering.

## Native assets

The managed package restores the matching `Microsoft.WebUI.Runtime.<rid>`
package transitively. Use `WEBUI_LIB_PATH` only when testing a custom local
native build.

See [Streaming Boundaries](/learn/directives/boundary) and the
[C ABI](./ffi) for the shared contract.
