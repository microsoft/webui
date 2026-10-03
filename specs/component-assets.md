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

## Publication lifetime

CLI and library writers share dependency-first, per-root atomic publication.
Immutable payload filenames bind their bytes; existing content-addressed paths
must compare equal or publication fails. Unchanged files are not staged or
replaced. A finalizer failure restores prior roots and bookkeeping. Payloads
stay available even if a reader observed a root during a rolled-back update.

Hosts serialize writers to one directory. Immutable dependencies remain on disk
until the owner explicitly establishes that readers of older roots have
finished. `prune` removes only obsolete compiler-owned payload paths and requires
reader/publisher quiescence. No time interval or generation count guarantees
that property. Persistent disk usage therefore grows until coordinated cleanup;
there is no automatic finite disk bound for arbitrary uncoordinated readers.

The CLI exposes this operation as `prune-component-assets`, requiring an
explicit output directory and `--quiescent` acknowledgment. It does not establish
quiescence itself and refuses cleanup without existing, parseable publication
bookkeeping.

The development HTTP server retains current root descriptors in memory and
reads immutable dependencies from the publication directory. Without an
explicit output directory, a server-owned temporary directory lasts until
shutdown. There is no resident cache of historical dependency generations.
