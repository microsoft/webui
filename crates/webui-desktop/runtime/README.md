# Windows App SDK bootstrap deployment

The `win-x64`, `win-arm64`, and `win-x86` directories contain only the
unmodified Microsoft bootstrap loader, not a self-contained shared framework.
The DLLs are governed by `Microsoft.WindowsAppSDK.LICENSE.txt`, **not MIT**.
Redistributors must comply with its distribution requirements; the accompanying
`Microsoft.WindowsAppSDK.NOTICES.txt` explains the deployment prerequisites.
`Microsoft.WindowsAppSDK.PROVENANCE.json` records the pinned official NuGet
archive and individual file hashes.

Restore or verify the vendored inputs from the repository root:

```powershell
python scripts\acquire-windows-app-sdk.py
```

The acquisition script uses only Python's standard library and downloads only
to `target\windows-app-sdk`. It verifies the pinned archive before extracting
the three bootstrap DLLs and original license. Ordinary Cargo builds are
offline and do not run this script. Include this directory in the published
Cargo crate.

Windows builds with `native` enabled stage the target architecture's bootstrap,
license, notices, and provenance alongside Cargo profile executables and
`deps` test executables. Unchanged files are not rewritten, so an existing
running process does not lock out a no-op build. Other operating systems and
builds without `native` do not stage these files.

Every `WindowsPortable` package requires those four sibling files beside its
runner, including when using a custom external runner. This explicit policy
prevents accepting an SDK runner whose dynamically loaded bootstrap was lost.
Custom non-SDK runners must supply these deployment files or use an external
packager. Linux and macOS packaging never copies these Windows assets.
Move the runner and all four files together; installing or copying just the
runner executable is insufficient. The destination also needs the installed
matching-architecture Windows App Runtime 1.8 and WebView2 Runtime.
