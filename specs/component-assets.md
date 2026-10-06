# Component asset boundary

The generated ESM graph is an input to the application bundler, not a second
deployment or chunk loader. Only stable root paths belong in authored imports.

## Version 4 payloads

A root default-exports a versioned `webui-component-asset` object with `root`,
`externalComponents`, static `imports`, and `componentStyles`. Each imported
payload contains exactly one entry in `templates`, its `componentStyles`, and
optional `templateFunctions`. Payloads have no independent version, kind,
component inventory, prerequisites, or imports. The root's required template
set is the disjoint union of imported template names and external components;
the root itself belongs to that union.

The compiler partitions the conservative template closure against entry
ownership. Every imported payload has template metadata. Every resource named
in an owned style closure exists and belongs to that root's required component
set. External style resources needed by those closures are included in the root
catalog, even if the entry's CSS delivery groups them differently.

Style closures preserve compiler source/cascade order. Light descendants
contribute to the current tree; Shadow descendants start another tree. Sorted
ESM imports do not define CSS order.

The trusted runtime checks external template availability and conflicts with
the live style catalog, including already registered imported components.
Condition closures and Trusted Types preparation complete before registry
mutation. CSS readiness precedes template publication and element creation.
Manifest-driven loading additionally validates payload shape, disjoint
providers, within-graph style consistency, and closure-resource coverage.

One imported root object owns one runtime facade. Registration failures can
retry. Native ESM import failures are subject to the browser's module-map
lifetime; application promise eviction is not module-cache invalidation.
Applications own bundler-specific recovery or explicit document reload.
Speculation never triggers a reload.

## Publication

CLI and library writers share dependency-first publication. Immutable payload
filenames bind their bytes; existing content-addressed paths must compare equal
or publication fails. Unchanged files are not staged or replaced. A finalizer
failure restores stable roots changed by that publication. Immutable payloads
already made visible remain available to concurrent readers. Hosts must
serialize publishers targeting the same generated input directory.

The generated input directory is owned by the application build. Publication
overwrites current stable roots and writes required content-addressed payloads,
but never deletes older generated inputs or unrelated files. Build pipelines
that require a clean directory remove and recreate their disposable output
before building.

The development HTTP server retains current root descriptors in memory and
reads payloads from the generated input directory. Without an explicit output
directory, a server-owned temporary directory lasts until shutdown.
