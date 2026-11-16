# Native SDK distribution contract

Standalone language-neutral release archives distribute the version-matched
C ABI SDK. Release staging owns `publish/sdk-contents/<target>`; neither npm
packages nor `node_modules` carry SDK payloads. Version identity comes from
the workspace release metadata, not an npm manifest.

Standalone archives contain the SDK root directly, not a second application
or package-manager wrapper. Each archive has an adjacent SHA-256 file. Normal
matrix aggregation retains each platform's SDK; final release validation
requires every supported platform's complete archive and checksum.

## Artifact identity and integrity

A complete SDK contains the generated public header, its platform's dynamic
library, license notices, and a metadata inventory. Windows additionally
contains the import library required to link against that DLL.
`xtask/src/publish_sdk.rs` defines staging and metadata validation.

Compiler, protocol consumer, header, and native library must belong to the same
WebUI release. Staging validates SDK metadata:
version, architecture, operating system, target ABI, and complete file hashes.
An explicit SDK never falls back to another installed copy after validation
fails. Missing or unsupported artifacts are actionable errors, not an implicit
download, source build, or version substitution.

Hashes detect corruption and mismatched staging; they are not an independent
signature or authorization boundary. Distribution authenticity remains the
responsibility of the approved release process.

SDK metadata is computed after all library transformations. Native consumers
must not depend on paths from the build machine. The macOS library's install
identity is relative to the consumer's runtime search path. On Windows, the
import library and runtime DLL must describe the same ABI.

## Build and runtime ownership

Consumers supply explicit include, link, and loader paths through their native
toolchain and deployment configuration. SDK acquisition is explicit; the
binding does not install compilers or mutate global search paths.

Deployed hosts load the compiled protocol once, keep handles alive through
active requests, and dispose results through the C ABI's designated functions.
Only public browser assets are HTTP resources. The protocol, SDK, source files,
and package metadata are not public static files.

The SDK does not make NuGet publication implicitly available or approved.
Managed package consumers still require the release's complete, approved
Microsoft.WebUI package source. Publication policy and platform acceptance
remain release gates.

## Go module publication

The public `github.com/microsoft/webui/go/v3` source module contains the Go
binding and source tests, not native libraries or generated protocols. Its
`Version` constant matches the compiler/native SDK, including native hotfix
suffixes. Stable Go module versions match that identity. Native hotfix
`X.Y.Z-hotfix.N` maps to Go module version `vX.Y.(Z+1)-0.hotfix.N`, which Go
orders after stable `vX.Y.Z`, before stable `vX.Y.(Z+1)`, and numerically by
hotfix number. Mapping fails if incrementing the patch would overflow.
Native SDK selection always uses `Version`, not the mapped module version.
Standard Go module publication uses the `go/v<Go version>` tag prefix for the
repository's `go/` subdirectory; the root `v<native version>` tag does not
publish that module.
Changing the release's major version requires the corresponding Go module
path and self-import migration before staging can succeed.
Root and Go tags are preflighted together and missing refs are pushed
atomically. Existing remote tags must resolve to the selected release commit,
including after a concurrent publisher wins a race.

Prepublication staging also emits Go proxy files with the canonical module
path, semantic version, and zip prefix. These source artifacts support
independent consumer validation without `replace` directives, workspace links,
or modifying a user's Go proxy/checksum configuration. They do not publish
Git tags or bypass the public module checksum database. Checks that use an
isolated file proxy may disable public checksum lookup only for that local
test environment.

The binding resolves headers/libraries through the caller's native toolchain,
not Go module initialization or implicit network downloads. Go ownership
and error behavior live with the package source and tests; SDK identity
and archive contracts live in release staging.
