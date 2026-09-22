# WebUI Desktop Build

Build-time typed protobuf IPC generation for `microsoft-webui-desktop`.
This crate is a build dependency, not part of the desktop runtime graph.

## Generate

Install `protoc`, `ts-proto@2.12.3`, and `@bufbuild/protobuf@2.15.0` explicitly.
The generator checks the installed ts-proto/runtime pair and never downloads
tools. The SDK options schema is embedded in this crate and automatically
included; installed CLI consumers do not need the crate's source directory.

```rust,no_run
use webui_desktop_build::{generate, GenerateConfig};

let generated = generate(&GenerateConfig {
    roots: vec!["schema/application.proto".into()],
    includes: vec!["schema".into()],
    rust_out: "src/generated".into(),
    ts_out: "client/generated".into(),
    lock_file: "schema/ipc-schema.lock.json".into(),
    check: false,
    protoc: None, // protoc on PATH, or an explicit executable
    ts_proto_plugin: Some("node_modules/.bin/protoc-gen-ts_proto".into()),
})?;
# Ok::<(), webui_desktop_build::GenerateError>(())
```

Create the lock file's parent directory first. Relative paths are relative to
the invoking process's working directory. Inputs and import directories are
canonicalized, and command arguments preserve paths containing spaces.

Application schemas import `webui/ipc/options.proto`, declare a positive
`contract_major` and nonempty `contract_name`, and set each service's receiver
to `HOST` or `RENDERER`. Every method needs a globally unique explicit ID above
1023. Notifications require both `notification = true` and the response marker
`webui.ipc.Notification`. A response of `google.protobuf.Empty` instead
generates an acknowledged RPC returning Rust `()` / TypeScript `void`.

## Outputs

- Rust: `ipc.rs`, `ipc_messages.rs`, and prost's package modules.
  Include `ipc.rs` in an application module. `messages` contains the package
  tree; service-named modules contain `Rpc` / `Event` markers.
  `HostHandler` / `register_host` and `RendererClient` are generated from
  correspondingly named services. The host trait includes both RPC methods
  (`RequestContext`) and notification methods (`NotificationContext`).
  `register_host` preflights and installs both kinds before frame startup.
  Notification definitions are installed for each admitted document and count
  toward callback limits. Optional dynamic host event subscriptions remain in
  the service module, such as `host::subscribe_selected`.
- TypeScript: `ipc.ts`, application codec modules, and the Empty codec when
  needed. `ipc.ts` exports `connectDesktop`, `HostClient`, `RendererHandlers`,
  `RendererEvents`, `AppConnection`, and schema metadata. Message types are
  exported by their codec modules. Generated bindings import the browser-only
  `@microsoft/webui-desktop` package; bundle them with the application.
- Beside the lock: `ipc-schema.json` (normalized semantic schema) and
  `ipc-generated-files.json` (owned artifact inventory).
- `GeneratedFiles` returns the paths and common SHA-256 schema hash.

Commit the lock and generated outputs together. `check: true` recompiles and
compares all artifacts without rewriting them. Missing, changed, or obsolete
artifacts return `GenerateError::Drift`. Normal generation removes obsolete
modules listed in the prior inventory, without deleting unrelated files.

Every current and obsolete artifact, including the lock, manifest, and
inventory, is validated before publication. Output roots, artifact files, and
parents beneath those roots must be real directories/regular files, not
symlinks (including dangling or internal symlinks). Ancestors above the
configured roots are canonicalized. Invalid destinations return
`ipc-output-path`; distinct artifacts cannot resolve to one destination.

Generation stages every changed file beside its destination before replacing
anything. It retains originals through payload updates and obsolete-file
removals, then advances the lock and inventory last. A publication failure
rolls back completed operations, including newly created files/directories.
Changed artifacts are rechecked before replacement/removal; concurrent changes
return `ipc-output-changed` rather than being overwritten.

This is rollback on reported filesystem failures, **not atomic multi-file
visibility or power-loss/crash atomicity**. Do not run concurrent writers or
read a deployment while generating across multiple roots. Path checks are not
an OS-level defense against adversarial filesystem races. If rollback itself
fails, `ipc-publication-rollback` preserves staging backups for recovery.
Post-publication cleanup failures explicitly report `ipc-publication-cleanup`
with the committed state; they do not claim that publication was rolled back.

Schema names are also checked before publication. A renderer method that
normalizes to `new` conflicts with the generated client constructor, and
service/marker names that shadow generated modules or runtime types return
`ipc-name-collision`. Normal Rust keywords such as a method named `Type` are
escaped; colliding normalized names are rejected rather than emitting invalid
bindings.

## Types and bounds

Prost and ts-proto produce the binary codecs. Integers retain their full
protobuf ranges; JavaScript 64-bit fields are `bigint`, bytes are `Uint8Array`,
unknown enum numbers are retained, optional presence is preserved, and oneof
values use discriminated unions. Repeated fields and acyclic nested messages
are supported. Generated metadata drives the runtime's iterative object and
wire validators before codec use.

All protobuf maps use JavaScript `Map<K, V>` (`useMapType=true`), never object
dictionaries. String keys stay strings, bool keys stay booleans, 32-bit integer
keys are numbers, and all 64-bit integer keys are lossless bigints. Validators
check native key types and integer ranges without string coercion. Strings
such as `__proto__`, `constructor`, and `prototype` are ordinary safe Map keys.
Rust uses prost's `BTreeMap<K, V>` with the corresponding native key types.

Generation rejects proto2, application extensions, unrecognized custom options,
streaming, recursive message graphs, groups, and well-known types other than
Empty. The Notification marker cannot be used as application data. Graphs are
limited to depth 16, 4096 messages, and 65536 fields; root source and descriptor
files are capped at 16 MiB. Compiler diagnostics are capped at 16 KiB.

## Compatibility and failures

The schema hash excludes source locations/comments and sorts messages, fields,
enums, and method identities deterministically. Both endpoints require the
same hash and major. Method IDs are permanent: removal retires an ID, and a
retired ID cannot be reused. Intentional method renames require an explicit
lock-name update retaining the ID. Changing a method signature or field
presence requires a major increment. Existing protobuf field types cannot
change; removed fields and enum values must be reserved by number and name.
Reservation history is preserved, including temporarily disconnected types.

`GenerateError::code()` exposes stable diagnostic codes. Schema diagnostics
carry qualified descriptor context and actionable help; protoc failures retain
its file/line diagnostic. Tool and filesystem failures are explicit and do not
silently fall back to another compiler or application DTO format.

## Targeted verification

```sh
cargo test -p microsoft-webui-desktop-build
node crates/webui-desktop-build/tests/run-typescript.mjs
node crates/webui-desktop-build/tests/run-rust.mjs
```

To deliberately regenerate the shared checked-in test fixture:

```sh
WEBUI_UPDATE_IPC_FIXTURE=1 cargo test -p microsoft-webui-desktop-build --test generate shared_fixture_matches_generator
WEBUI_UPDATE_IPC_FIXTURE=1 cargo test -p microsoft-webui-desktop-build --test codecs
WEBUI_UPDATE_IPC_FIXTURE=1 node crates/webui-desktop-build/tests/run-typescript.mjs
```
