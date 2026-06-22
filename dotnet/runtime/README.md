# Microsoft.WebUI.Runtime

Native runtime packages for Microsoft.WebUI.

`Microsoft.WebUI.Runtime.<rid>` packages carry the RID-specific `webui_ffi` native library under `runtimes/<rid>/native`. They do not contain managed APIs. The managed `Microsoft.WebUI` package depends on these runtime packages so NuGet restores the native asset for each supported platform.

Most applications should install the managed package:

```bash
dotnet add package Microsoft.WebUI
```

Reference a runtime package directly only when you are manually assembling native assets for custom packaging or testing.

## Supported packages

| Runtime | Package |
|---------|---------|
| Windows x64 | `Microsoft.WebUI.Runtime.win-x64` |
| Windows ARM64 | `Microsoft.WebUI.Runtime.win-arm64` |
| Linux x64 | `Microsoft.WebUI.Runtime.linux-x64` |
| Linux ARM64 | `Microsoft.WebUI.Runtime.linux-arm64` |
| macOS x64 | `Microsoft.WebUI.Runtime.osx-x64` |
| macOS ARM64 | `Microsoft.WebUI.Runtime.osx-arm64` |

## Documentation

See the [WebUI repository](https://github.com/microsoft/webui) for full usage guides and examples.

## License

MIT. NuGet package metadata uses © Microsoft Corporation. All rights reserved.
