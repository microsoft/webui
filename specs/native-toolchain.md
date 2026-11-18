# Native application toolchain contract

This reference defines the boundary between native application orchestration,
browser compilation, embedded first-party resources, and the standalone native
TypeScript checker. The C ABI SDK has a separate
[distribution contract](native-sdk.md). Public commands and setup live in the
[CLI reference](../docs/reference/cli/index.md) and
[installation guide](../docs/start/installation.md).

## Application ownership

`webui init` uses native frontend tooling for every server choice.
Generation is offline and independent of installed host tools. Native
Rust/Go/C/C++/.NET applications contain no `package.json` or `scripts/*.mjs`;
Node/Bun retain runtime packages and `server/http.mjs`, not JavaScript build
orchestration. Optional restoration uses the selected host toolchain. Native
AI loader installation copies embedded reference resources without a network.

`webui.toml` schema 1 pins the exact WebUI release and selects the server,
frontend source directory, HTML entry, optional browser entry, host manifest,
native target name, and host source. Paths must be project-relative without
parent segments. Unknown fields and mismatched schema/release identities fail.
`commands/app/config.rs` owns the exact schema and host presets.

The shared app supervisor owns a single writer lock across build, dev, and
start. C/C++/.NET checks also take it because their host checks perform builds.
Check runs strict native TSC against the project's configuration when a browser
entry is configured, then checks the host. Build retains prepared templates,
bundles the browser graph, proves final bytes, stages protocol and public assets,
builds the host, then promotes the generation. Frontend-only build skips host
compilation for direct native-toolchain use. Start verifies the recorded
executable and protocol hashes and never compiles or restores dependencies.

Development watches bounded native filesystem events and coalesces edits.
It stops the host before rebuilding, does not launch a host from a build
superseded by an observed edit, and waits for another edit after a build error. Generated output
is ignored. Failure is never reported as a successful stale-generation fallback.
Interrupted promotion is an explicit recovery error, not permission to discard
the previous generation silently.

## Browser and resource boundary

The optional `webui::browser_build` module uses Rolldown 1.3.2; build-only
`webui-projection` uses Oxc 0.158.0 for the shared
[semantic proof](projection.md). Node's thin browser-build binding calls this
complete native pipeline through N-API; it owns no projection graph, manifest
builder, or optional compiler-peer compatibility layer. Press calls the native
builder directly. Native compilation uses neither a JavaScript VM nor a Node
subprocess. Handler crates and standalone native renderer hosts do not depend
on the compiler module or analysis crate. One `microsoft-webui-node` crate
produces two native artifacts in separate Cargo target roots, using mutually
exclusive `handler` and `build` features. The default `handler` variant has no
production compiler dependency. The `build` variant enables the optional
browser compiler in `microsoft-webui`. Both have the Cargo library name
`webui_node`; platform npm packages stage them as `webui.node` and
`webui-build.node`. They share one npm installation. The runtime root
`@microsoft/webui` exposes `Protocol`, streaming, inspection, and runtime types.
The explicit `@microsoft/webui/build.js` entry owns `build`, `prepareBuild`,
`bundleClient`, and compiler types; none are re-exported by the root.
Each entry loads only its own addon when used. `WEBUI_ADDON_PATH` overrides
runtime resolution; `WEBUI_BUILD_ADDON_PATH` independently overrides build
resolution. Platform npm packages ship both artifacts and the CLI install
remains in the same package.

The runtime binding depends on the handler without a production compiler
dependency. The compiler binding may use the same handler library internally,
but N-API handles never cross addon boundaries. Compilation passes serialized
protocol bytes to runtime consumers, not a loaded native `Protocol`.
Prepared templates compile once across bundling and finalization; final-byte
hashes and the existing protocol validator remain authoritative.

Standalone `bundle --watch` watches the configured project boundary
conservatively because the pinned resolver does not expose every inherited
configuration read. This is invalidation coverage, not proof of containment
for external inherited configuration. Exact outputs, manifests, locks and
publication staging are ignored without resetting source baselines or dropping
pending events. Explicit `--watch-ignore` paths declare additional exclusions;
native recursive subscription cost scales with the project tree.

The CLI's Rust-only build script stages canonical framework TypeScript source,
not a second renderer or prebuilt JavaScript frontend. Original package export
metadata maps to matching source files in
`.webui/toolchain/<release>/framework`. Native checker path mappings select
these same sources. Existing conflicting cache files fail rather than being
silently replaced. The same release cache owns offline reference Markdown and
its loader. `reference --path` materializes the core reference and adjacent
`docs/` without Node/npm.

Source checkout staging reads first-party resources directly. The published CLI
crate must include staged `assets/` with matching framework/reference data;
consumer Cargo builds must never fetch npm resources. Native browser assets
can be built by the source Rust CLI without Node; full application checking
additionally requires the genuine native TSC companion. Production disables
sourcemaps; development emits them. Legal comments remain inline, and public
output includes `webui-LICENSE.txt`.

## Standalone checker distribution

The archive name is
`webui-toolchain-<version>-<target-triple>.tar.gz`, with an adjacent SHA-256
sidecar. Its root contains:

```text
bin/webui[.exe]
compiler/tsc[.exe]
compiler/lib*.d.ts
notices/WebUI-LICENSE
notices/TypeScript-LICENSE
notices/TypeScript-NOTICE.txt
toolchain.json
```

WebUI release identity and TypeScript version are independent. The checker is
pinned to TypeScript **7.0.2**. Supported targets are x86_64/aarch64 macOS,
GNU Linux, and MSVC Windows. `toolchain.json` uses schema 1 fields
`schemaVersion`, `version`, `target`, `typescriptVersion`, and `files`, where
`files` maps the exact payload inventory (excluding the metadata file itself)
to lowercase SHA-256 hex strings.

`WEBUI_TOOLCHAIN_DIR`, when set, authoritatively selects the archive root.
Otherwise resolution selects the directory above the running CLI's `bin/`
directory. Empty or invalid overrides never fall back. The consumer rejects
unknown metadata fields, duplicate inventory keys, missing or extra entries,
symlinks, escaping paths, mismatched versions/target, and invalid hashes.
It validates native executable format/architecture for CLI and checker before
a bounded native `--version` probe. Metadata and artifact reads have explicit
size limits; file hashing is streamed.

There is no PATH `tsc` guessing, consumer `node_modules` search, download,
JavaScript launcher, or Node fallback. Archive hashes establish integrity, not
independent authenticity; approved release provenance establishes trust.
Producer-only acquisition uses an isolated Go-standard-library HTTPS helper
against official `microsoft/TypeScript` GitHub release assets. It verifies
TypeScript 7.0.2 against the exact frozen `pnpm-lock.yaml` SHA-512 before
extraction. Host-native fetch execution ignores foreign `GOOS`/`GOARCH`;
acquisition never runs Node/npm or target executables. Go is a producer
prerequisite, not a consumer prerequisite unless Go is the selected backend.

## Portable host lifetime

The application supervisor gates work inside an owned Unix process group or
Windows Job. Private host control opts in with `_WEBUI_APP_CONTROL=stdin-v1`;
one ASCII `S` or EOF requests graceful shutdown. This is a tooling/host
contract, not a render-engine option. Hosts stop admitting requests, drain
active requests, then release protocol/renderer resources.

The app supervisor has a 13-second outer deadline. Host drain allowance is up
to 10 seconds, with 11 seconds to confirm host exit. Rust's generated server
uses a 5-second drain timeout; .NET and Go use 10 seconds. Failed confirmation
is an error and triggers owned-process cleanup, not a successful restart.
Machine-facing `app --control-stdin` is separate from the private host pipe.
It is disabled by default. When enabled, the supervisor alone reads one ASCII
`S` or EOF as a graceful stop request, without console UI. A malformed byte
requests cooperative cleanup but remains an error after containment is joined.
Ordinary app commands do not implicitly consume stdin.

## Defining sources

- `crates/webui-cli/src/commands/{app,toolchain}/`
- `crates/webui-cli/src/commands/{native.rs,init/}`
- `crates/webui-cli/resources/build_support.rs`
- `crates/webui/src/browser_build/`, `crates/webui-projection/`
- `crates/webui-dev-server/src/shutdown/`
- `xtask/src/native_toolchain.rs`, `xtask/src/native_toolchain/`
