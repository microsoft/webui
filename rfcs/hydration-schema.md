# RFC: Build-Time Hydration Schema and Startup-State Projection

**Status:** Implemented
**Scope:** `webui-parser`, `webui`, `webui-protocol`, `webui-handler`, FFI, Node, WASM, .NET
**Breaking:** Yes - the initial `#webui-data.state` contains the active request's hydratable state, not the full server state.

## 1. Summary

WebUI keeps one consolidated `<script type="application/json" id="webui-data">`
bootstrap block, but projects its `state` field to the properties that reachable
components can hydrate for the current request path.

The build stores:

- a sorted, deduplicated `ComponentData.hydration_keys` list per component
- a sorted global `WebUIProtocol.hydration_schema` union for old protocols and
  conservative fallback

At render time the handler:

1. Finds components reachable from the active entry and route chain.
2. Unions only those components' hydration keys.
3. Projects the borrowed top-level state object with an adaptive iterator.
4. Serializes the projected object through the existing buffered script-safe
   JSON path.

Hosts can also prepare a protocol once and reuse its decoded protocol and
deterministic indices. This removes protobuf decoding and index construction
from repeated full, partial, component-template, and token requests.

## 2. What the Measurements Show

The original hypothesis was partly correct:

- In an isolated handler benchmark, creating `#webui-data.state` accounts for
  nearly all render time when the template itself is minimal.
- Projection is extremely effective when most state is not hydratable.
- If a large collection is itself hydratable, its JSON bytes still have to be
  serialized.
- In FFI, Node, WASM, and .NET, parsing the complete input JSON remains a major
  cost after output projection.

The implementation therefore addresses both output size and repeated protocol
startup work. It does not claim that projection removes the cost of parsing
request state.

## 3. Decisions

### 3.1 Keep one named state object

The client continues to consume `window.__webui.state`. Positional per-instance
payloads were rejected because they duplicate shared values and require a more
complex schema and client contract.

### 3.2 Store per-component keys and retain a global fallback

Each component carries its own hydration keys. The protocol also retains the
global union:

```proto
message ComponentData {
  string template = 1;
  string css = 2;
  string css_href = 3;
  string template_json = 4;
  string template_functions = 5;
  repeated string hydration_keys = 6;
}

message WebUIProtocol {
  map<string, FragmentList> fragments = 1;
  repeated string tokens = 2;
  map<string, ComponentData> components = 3;
  CssStrategy css_strategy = 4;
  DomStrategy dom_strategy = 5;
  repeated string hydration_schema = 6;
  uint32 hydration_schema_version = 7;
}
```

Protocols produced before `hydration_keys` existed fall back to
`hydration_schema`. Protocols produced before hydration projection have
`hydration_schema_version = 0` and preserve full-state emission; current builds
write version `1`, so an empty schema intentionally emits `{}`.

### 3.3 Scope projection to the active request

At `body_end`, the handler already computes reachable components for template
and CSS delivery. The same set now selects hydration keys. Inactive sibling
routes are excluded, while components behind active-route `<if>` and `<for>`
branches remain conservatively reachable.

This keeps initial state aligned with the templates the request can hydrate
without dropping values that may become visible after a client-side condition
changes.

### 3.4 Iterate the smaller side

`ProjectedState` chooses between:

- iterating the schema and calling `map.get()` when the schema is smaller
- iterating the state map and binary-searching the sorted schema otherwise

This preserves the compact-state path and avoids scanning very wide state
objects for a sparse schema. Duplicate schema entries are handled
defensively.

### 3.5 Keep buffered script-safe serialization

The accepted implementation does not stream `serde_json` directly into the
response writer. A direct streaming adapter was measured and regressed the
equal-byte 1 MB path.

The handler serializes the projected object into a reusable-sized buffer,
validates UTF-8, and performs the script-safe `</` escaping pass. Projection
reduces the number of serialized bytes; it does not replace the proven
serializer with a slower one.

### 3.6 Lexically scan component scripts

`scan_hydration_attributes` is iterative and does not use regular expressions
or recursion. It recognizes top-level `@observable` and `@attr` decorators
while skipping:

- line and block comments
- quoted strings
- template literal text and nested `${...}` expressions
- regular-expression literals

After a decorator, it handles option groups, stacked decorators, TypeScript
member modifiers, accessors, and comments before the property name.

### 3.7 Reuse prepared protocols in hosts

`PreparedProtocol` owns a decoded `WebUIProtocol`, an immutable component
index, and a lazily populated template-metadata cache. Partial and action
requests use request-local route caches because resolved relative routes can
contain request parameter values. Only individual metadata-cache lookups use a
read-write lock, so concurrent prepared requests do not serialize.

Surfaces:

- Rust: `webui_handler::PreparedProtocol`
- C: `webui_protocol_create` / `webui_protocol_destroy` and prepared render
  functions
- Node: an internal native `PreparedProtocol`, cached transparently per
  `Buffer` by the npm package
- WASM: exported `PreparedProtocol` class
- .NET: public `PreparedProtocol : IDisposable` backed by a `SafeHandle`

### 3.8 Avoid partial-response state reserialization

Partial hosts previously parsed `state_json` into `serde_json::Value`, inserted
it into the response object, then serialized the same state again.

The new path validates state with a streaming serde visitor that enforces
`serde_json::Value` numeric limits, then embeds the original JSON bytes into the
serialized response. Invalid or trailing JSON is still rejected.

## 4. Build-Time Hydration Surface

The WebUI parser plugin owns hydration extraction. For each component it unions:

- sibling `.ts` / `.js` `@observable` and `@attr` properties
- template reactive roots and observed attributes already compiled into WebUI
  template metadata

The result is sorted and deduplicated before being attached to
`ComponentTemplateArtifact` and then `ComponentData.hydration_keys`.

Precompiled package components without source modules contribute their template
surface only. A package format can carry explicit hydration keys in the future.

## 5. Runtime Flow

```text
build
  component script + template roots
    -> ComponentData.hydration_keys
    -> WebUIProtocol.hydration_schema fallback

full render
  active entry + request path
    -> reachable components
    -> request hydration schema
    -> adaptive projection
    -> script-safe #webui-data.state

repeated host request
  PreparedProtocol
    -> decoded protocol reused
    -> component index reused
    -> request route cache reset where required
```

## 6. Security Model

Hydration projection is a payload and performance boundary, not a secrecy
boundary.

Skipping comments and literals prevents false decorator matches from expanding
the client payload. It does not make arbitrary server state safe to expose.
Template roots and authored decorators intentionally place values in the
client-facing schema.

Hosts must never place credentials, private tokens, or other secrets in state
that can be rendered to the browser. Tests verify that commented and
stringified decorator text does not add a key, but documentation must not claim
that over-inclusion is harmless or that leakage is impossible.

## 7. Performance Evidence

Measurements use Criterion confidence intervals for Rust paths and repeated
release-mode rounds with warmup and medians for host paths.

| Change | Before | After | Result |
|---|---:|---:|---:|
| Scanner, 256 noisy blocks | baseline | 27.68 us | 80.6% faster |
| Wide 30,000-key projection | about 194 us | 0.79 us | 99.6% faster |
| Dashboard excluding inactive contacts | about 97 us | 4.01 us | 95.9% faster |
| Active contacts, scoped vs global in same run | 105.44 us | 105.21 us | no regression |
| Partial state, 64 KB | 254.54 us | 37.93 us | 85.1% faster |
| Partial state, 1 MB | 5.716 ms | 1.054 ms | 81.6% faster |
| Shared prepared partial, 8 threads x 10,000 | 2.059 s | 0.203 s | 90.1% faster |
| FFI full render, 100-component protocol | 48.34 us | 0.458 us | 99.1% faster |
| FFI partial, 100-component protocol | 57.33 us | 1.594 us | 97.2% faster |
| Node contact app object workflow | 166.59 us | 125.45 us | 24.7% faster |
| WASM contact app object workflow | 203.52 us | 149.70 us | 26.4% faster |
| .NET contact app full render | 196.85 us | 108.03 us | 45.1% faster |
| .NET contact app partial render | 130.05 us | 38.98 us | 70.0% faster |

Criterion baseline comparisons showed machine-frequency drift during some
short runs. Decisions use large deltas plus same-process controls:

- the equal-byte 1 MB arm guards the existing serializer
- the active contacts route is compared directly with a legacy global-schema
  protocol in the same benchmark process
- prepared and legacy host entry points use the same fixture and process

## 8. Rejected Approaches

### Direct streaming state serialization

Rejected because it made the equal-byte 1 MB case materially slower. The
buffered serializer remains.

### Map-first projection for every state

Rejected because sparse schemas over wide maps scanned every server-only key.
Adaptive iteration wins without changing output.

### Global-only projection

Rejected because inactive routes can own most of the state. Request reachability
provides a much tighter safe subset.

### Direct JavaScript-object conversion in Node and WASM

Rejected after measurement:

- Node direct object conversion: 220.79 us
- Node stringify plus prepared JSON render: 121.70 us
- WASM direct object conversion: 200.86 us
- WASM stringify plus prepared JSON render: 149.70 us

The public JavaScript workflow keeps `JSON.stringify` and removes the measured
regression.

### Retaining route-cache entries across requests

Rejected because nested relative routes can generate cache keys containing
actual parameter values. Reusing those entries would allow unbounded growth.

## 9. Acceptance Criteria

- Commented or literal decorator text does not contribute hydration keys.
- Wide sparse projection does not scan the complete state map.
- Initial projection excludes inactive sibling routes.
- Active routes retain every required hydration key.
- Legacy protocols without per-component keys use the global schema.
- Full-state equal-byte performance does not regress.
- Prepared host APIs preserve legacy output and error behavior.
- Partial state is validated without building and serializing a duplicate tree.
- `DESIGN.md`, integration docs, and benchmarks describe the shipped behavior.
