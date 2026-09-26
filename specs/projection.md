# State projection contract

This is the internal, cross-language contract between a bundler adapter, the
build-only projection compiler, and the Rust WebUI build. See
[DESIGN.md](../DESIGN.md#optional-state-projection) for why projection exists;
see the [hydration guide](../docs/guide/concepts/hydration.md#build-time-state-projection)
for application usage. The manifest is a **proof of one completed bundler
build**, not a request-time input or a secrecy boundary.

## Producer and consumer responsibilities

The adapter reports the bundler's resolved module graph, exact authored import
edges and resolved targets, final output-to-input membership after tree shaking,
and the bytes of every physical output. It must not infer membership from output
text or let the compiler resolve modules a second time. A component whose
defining module is absent from all final outputs does not appear in the
manifest. The compiler identifies only proven WebUI component declarations
and exact state keys; an unproven scripted component is a build error when
manifests are selected.

The TypeScript compiler emits a `webui.state-projection/v1` JSON fragment
with producer and adapter identities, `root`, `analysisHash`, `buildId`,
`inputs`, `outputs`, `components`, and optional `entryClosures`. Each component
names its defining module, final output set, initial hydration keys, and
navigation keys. Navigation keys include hydration keys; WebUI adds compiled
template roots when building the protocol. A scriptless component needs no
JavaScript analysis. Without a manifest, unknown client surfaces keep full
state for correctness; **when manifests are supplied**, every compiled
scripted component requires exactly one proven owner.

Each fragment is validated independently before merging. Identical component
tags in separate fragments are an error, not a union of guessed key sets.
Physical file identity is resolved relative to each fragment's own canonical
root. Conflicting hashes for the same file are an error even if both fragments
were individually valid. The Rust build rechecks declared input and output
bytes against disk before using projection data; the handler never loads
manifests or bundle files per request.

### Exact state-key proof

The compiler resolves symbol identity through the adapter's import/re-export
graph, including aliases and namespaces, rather than matching names or
resolving modules from the filesystem again. It associates a class with a
literal custom-element tag only through a proven `Class.define(tag)` or
unshadowed `customElements.define(tag, Class)` call. An unrelated `.define()`
is ignored; mutable class references and dynamic tags cannot prove an exact
result. Inherited properties are followed through the symbol graph. Proven
framework `@observable` and `@attr` properties contribute their **JavaScript
property names**, not renamed HTML attribute names, to the exact key set.
Unrelated decorators are ignored, but ambiguous framework decorators,
unresolved bases, and unsupported reactive property forms fail the build
instead of emitting a guessed or partial manifest. A proven empty key set is
valid. The final output-membership check then excludes tree-shaken classes.

## Canonical paths and hashes

- `root` is `"."` or a sequence of parent-only `..` segments relative to the
  manifest directory. Physical input, output, and module keys are relative to
  that root, use `/`, and cannot escape it. Canonicalized file access must
  remain within the root, including through symlinks.
- Virtual module and output IDs start with a NUL in the adapter graph. Manifest
  keys encode the UTF-8 bytes *after* that NUL as `virtual:` followed by
  lowercase hex; only these keys may carry the literal hash `"virtual"`.
- Physical inputs hash their source UTF-8 bytes; physical outputs hash their
  exact emitted bytes. Hashes are `sha256:` plus 64 lowercase hex digits.
  Object keys, component output lists, and component key lists sort by raw
  UTF-8 bytes, not locale. Lists of keys and outputs are unique.
- An `entryClosures` value lists the *other* outputs transitively imported by
  one entry through static imports, in the bundler's load order (largest
  first). Dynamic imports are excluded. Unlike keys, closure values must
  **not** be sorted by the consumer. Empty closures can identify an entry
  unambiguously when fragments are merged.

## Cross-language identity

`analysisHash` hashes the normalized semantic graph, not raw JSON. Its
canonical records, in order, are `entries` (count), sorted `entry` IDs,
`modules` (count), sorted `module` records, `memberships` (count), and sorted
`membership` records. A `module` covers its ID, kind, owning package or empty
string, source hash, import count, and each sorted import's authored specifier,
resolved ID or empty string, external/internal flag, static/dynamic kind, and
owning package or empty string. A `membership` covers the output ID, member
count, and sorted contributing module IDs. Import edges sort by their
UTF-8-ordered composite key of those five edge properties, separated by NUL.

`buildId` hashes the entire declared proof. Append the following records in
order, writing each field as `<UTF-8 byte length>:<field>` after its literal
label and ending the record with LF:

```text
schema(schema-id)
producer(name, version)
adapter(name, bundler@version)
root(root)
analysis(analysisHash)
inputs(count)
input(path, hash) ... sorted by UTF-8 path bytes
outputs(count)
output(path, hash) ... sorted by UTF-8 path bytes
components(count)
component(tag, module, output-count, outputs...,
          hydration-key-count, hydration-keys...,
          navigation-key-count, navigation-keys...)
          ... sorted by UTF-8 tag bytes
entryClosures(count) ... only when nonempty
entryClosure(entry, member-count, members...) ... sorted by entry key
```

Hash the concatenated record bytes with SHA-256 and prefix the lowercase
digest with `sha256:`. This is **not** a hash of the JSON serialization.
Omitting `entryClosures` when empty also omits its hash records so earlier
fragments retain their original build IDs. The Rust and TypeScript golden
vector for the same Unicode-bearing manifest is
`sha256:8319202a060626c39cce76df50197c92dee27aab29d601161183c188204d7c18`.
Do not change canonicalization without changing both producer and validator
and their shared vector.

## Defining sources

- [Adapter graph and membership interfaces](../packages/webui/src/projection/graph.ts)
  and [compiler graph normalization](../packages/webui/src/projection/compiler.ts).
- [Manifest shape and canonical hash](../packages/webui/src/projection/manifest.ts),
  [Rust validator](../crates/webui-protocol/src/projection_manifest.rs).
- [TypeScript golden-vector tests](../packages/webui/test/projection-compiler.test.ts),
  [Rust golden-vector tests](../crates/webui-protocol/src/projection_manifest.rs),
  and [adapter conformance tests](../packages/webui/test/projection-conformance.test.ts).
