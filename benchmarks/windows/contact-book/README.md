# Contact Book Windows desktop benchmark

This harness compares the release Electron and WebUI desktop hosts using the
same Contact Book bundle, state, 1200x800 window, and Dashboard readiness
contract. It uses WebView2/Electron CDP for readiness, Win32 window discovery
for launcher-to-host-main timing, and Windows Job Objects plus process-memory
sampling for lifecycle CPU and host-process RSS.

The native host resolves the manifest theme at startup. The harness gives
Electron a per-run copy of the same immutable state with the same theme tokens
injected; the checked-in state, application bundle, and readiness probe inputs
are not modified.

## Prerequisites

- Windows with the WebView2 Runtime installed.
- Node.js, pnpm, Python 3.11+, and the repository dependencies installed.
- A release build of `microsoft-webui-node`.
- A release build of `contact-book-desktop` with the `source` feature.
- The Contact Book app and Electron launcher built in release-compatible mode.

From the repository root:

```powershell
pnpm install --frozen-lockfile
pnpm --dir examples\app\contact-book-manager run build
pnpm --dir examples\integration\electron run build
cargo build --release -p microsoft-webui-node
cargo build --release -p microsoft-webui-cli
cargo build --release -p contact-book-desktop --features source
.\target\release\webui.exe build .\examples\app\contact-book-manager\src `
  --plugin=webui `
  --projection-manifest .\examples\app\contact-book-manager\dist\webui-projection.json `
  --out .\examples\app\contact-book-manager\dist
```

Run one warmup per host followed by 20 alternating Electron/WebUI pairs:

```powershell
$env:TEMP = "$env:SystemDrive\webui-bench-temp"
$env:TMP = $env:TEMP
New-Item -ItemType Directory -Force $env:TEMP | Out-Null
python benchmarks\windows\contact-book\run.py
```

The default output is written to
`benchmarks/windows/contact-book/results/windows-raw.json` and
`benchmarks/windows/contact-book/results/windows-summary.json`. Override the
output directory with `--output-dir`. Keep the temporary root short because
WebView2 appends its app identity and profile directories below it.

The analyzer uses the modified z-score for each metric, with
`abs(score) > 3.5` as the outlier rule. A pair is excluded when either host is
an outlier in its own host cohort for that metric. No sample is removed
manually.

The harness does not edit the application bundle, readiness probe, data, or
host implementation. WebView2 receives only a per-run remote-debugging port
through `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`; each host receives a fresh
profile directory.
