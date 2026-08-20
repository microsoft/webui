# .NET

`Microsoft.WebUI` wraps the native C ABI with safe handles and managed result
types. It targets .NET 8 and .NET 9.

## Installation

```bash
dotnet add package Microsoft.WebUI
```

Load the protocol once and reuse it:

```csharp
using var protocol = new Protocol(
    await File.ReadAllBytesAsync("dist/protocol.bin"));
using var handler = new WebUIHandler("webui");

string html = handler.Render(
    protocol,
    """{"title":"Home"}""",
    "index.html",
    "/");
```

## Progressive streaming

`StreamResponse` creates a single-driver session. `Start` and `Resume` return a
`StreamingStep` containing `Bytes`, `Done`, and an optional `Boundary`
descriptor.

```csharp
using var session = handler.StreamResponse(protocol, "index.html", "/");

Response.ContentType = "text/html; charset=utf-8";
StreamingStep step = session.Start(initialStateJson);

while (true)
{
    await Response.Body.WriteAsync(step.Bytes);
    await Response.Body.FlushAsync();
    if (step.Done) break;

    BoundaryDescriptor boundary = step.Boundary
        ?? throw new InvalidOperationException("Missing boundary descriptor");
    string state = await LoadBoundaryStateAsync(
        boundary.Owner,
        boundary.Name,
        boundary.Key);
    step = session.Resume(
        boundary.InstanceId,
        state,
        BoundaryMode.Final);
}
```

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
rerun hydration. The step that reports `Done` already includes the response
tail and terminal record.

Drive one session from one request flow at a time. Independent sessions may run
concurrently against the same handler and protocol. `WebUIException` carries
the native diagnostic for invalid state, ordering, keys, or rendering.

## Native assets

The managed package restores the matching `Microsoft.WebUI.Runtime.<rid>`
package transitively. Use `WEBUI_LIB_PATH` only when testing a custom local
native build.

See [Streaming Boundaries](/guide/concepts/directives/boundary) and the
[C ABI](./ffi) for the shared contract.
