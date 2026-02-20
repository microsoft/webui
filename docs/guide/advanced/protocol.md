# Protocol

The WebUI protocol is serialized as **Protocol Buffers (protobuf) binary format** for optimal runtime performance. The compiled output (`protocol.bin`) is consumed by [platform handlers](/guide/concepts/handlers/) to render HTML with application state.

## Proto Schema

The schema is defined in `crates/webui-protocol/proto/webui.proto`.

### Top-Level Message

```protobuf
message WebUIProtocol {
  map<string, FragmentList> fragments = 1;
}

message FragmentList {
  repeated WebUIFragment fragments = 1;
}
```

The `fragments` map keys are fragment identifiers (e.g., `"index.html"`, `"for-1"`, `"my-card"`). Each maps to an ordered list of `WebUIFragment` entries.

### Fragment Types

`WebUIFragment` uses a `oneof` to represent the different node types:

| Type | Description |
|------|-------------|
| `Raw` | Static HTML content (literal strings) |
| `Component` | A web component reference with a `fragment_id` |
| `For` | Loop directive with `item`, `collection`, and a child `fragment_id` |
| `Signal` | A reactive binding (`value`, `raw` flag) |
| `If` | Conditional rendering with a `Condition` and child `fragment_id` |

### Condition Expressions

The `If` fragment uses a `Condition` oneof to express its predicate:

| Variant | Description |
|---------|-------------|
| `Predicate` | Simple truthy check on an identifier |
| `Not` | Logical negation of a nested condition |
| `Compound` | Logical AND/OR of two nested conditions |
| `Identifier` | Direct reference to a state identifier |

## JSON Compatibility

JSON serialization is still supported for backward compatibility and debugging. Use the JSON representation when inspecting protocol output manually or integrating with tools that do not support protobuf.