// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;

pub(super) fn validate_message_descriptor(desc: &MessageDescriptor) -> Result<(), GenerateError> {
    let name = desc.full_name();
    if name == NOTIFICATION || !application_file(desc.parent_file().name()) && name != EMPTY {
        return Err(schema("ipc-unsupported-message", name, "message type is unsupported as application data", "use ordinary proto3 messages; only google.protobuf.Empty is supported from well-known types"));
    }
    if desc.extensions().len() != 0 || desc.extension_ranges().len() != 0 {
        return Err(schema(
            "ipc-extension",
            name,
            "message extensions are unsupported",
            "use ordinary proto3 fields",
        ));
    }
    allowed_options(desc.options())?;
    for oneof in desc.oneofs() {
        allowed_options(oneof.options())?;
    }
    for field in desc.fields() {
        allowed_options(field.options())?;
        if let Kind::Message(child) = field.kind() {
            if child.full_name() == NOTIFICATION
                || !application_file(child.parent_file().name()) && child.full_name() != EMPTY
            {
                return Err(schema(
                    "ipc-unsupported-message",
                    field.full_name(),
                    "unsupported application field type",
                    "use ordinary proto3 messages or google.protobuf.Empty",
                ));
            }
        }
        if field.is_group() {
            return Err(schema(
                "ipc-group",
                field.full_name(),
                "groups are unsupported",
                "use a nested proto3 message",
            ));
        }
        if field.kind() == Kind::Message(desc.clone()) {
            return Err(schema(
                "ipc-recursion",
                field.full_name(),
                "recursive messages are unsupported",
                "use an acyclic message graph",
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_all_messages(pool: &DescriptorPool) -> Result<(), GenerateError> {
    let mut messages = Vec::new();
    let mut fields = 0usize;
    let mut symbols = BTreeSet::new();
    for desc in pool
        .all_messages()
        .filter(|m| application_file(m.parent_file().name()) || m.full_name() == EMPTY)
    {
        validate_message_descriptor(&desc)?;
        fields += desc.fields().len();
        if fields > 65536 || messages.len() >= 4096 {
            return Err(schema(
                "ipc-schema-limit",
                desc.full_name(),
                "schema exceeds 4096 messages or 65536 fields",
                "split the contract into smaller schemas",
            ));
        }
        let msg = message(&desc);
        if !symbols.insert(msg.rust.clone()) {
            return Err(schema(
                "ipc-name-collision",
                desc.full_name(),
                "generated Rust message symbols collide",
                "rename one of the protobuf messages",
            ));
        }
        let mut field_names = BTreeSet::new();
        for field in &msg.fields {
            if !field_names.insert(names::ts_field(&field.name)) {
                return Err(schema(
                    "ipc-name-collision",
                    desc.full_name(),
                    "generated TypeScript properties collide",
                    "rename the conflicting fields",
                ));
            }
        }
        messages.push(msg);
    }
    for enumeration in pool
        .all_enums()
        .filter(|e| application_file(e.parent_file().name()))
    {
        allowed_options(enumeration.options())?;
        for value in enumeration.values() {
            allowed_options(value.options())?;
        }
    }
    validate_topology(&Contract {
        name: String::new(),
        major: 0,
        methods: Vec::new(),
        messages,
        enums: Vec::new(),
    })
}

pub(super) fn validate_topology(contract: &Contract) -> Result<(), GenerateError> {
    let by_name: BTreeMap<_, _> = contract
        .messages
        .iter()
        .map(|m| (m.name.as_str(), m))
        .collect();
    let mut done = BTreeMap::<&str, usize>::new();
    for root in &contract.messages {
        let mut visiting = BTreeSet::new();
        let mut stack = vec![(root.name.as_str(), false)];
        while let Some((name, exit)) = stack.pop() {
            if done.contains_key(name) {
                continue;
            }
            let message = by_name.get(name).ok_or_else(|| {
                schema(
                    "ipc-message",
                    name,
                    "referenced message is outside the supported graph",
                    "use supported application message types",
                )
            })?;
            if exit {
                visiting.remove(name);
                let mut depth = 1;
                for child in message
                    .fields
                    .iter()
                    .filter(|f| f.kind == "message")
                    .filter_map(|f| f.target.as_deref())
                {
                    let child_depth = done.get(child).ok_or_else(|| {
                        schema(
                            "ipc-recursion",
                            child,
                            "cyclic message graph",
                            "use acyclic messages",
                        )
                    })?;
                    depth = depth.max(child_depth + 1);
                }
                if depth > 16 {
                    return Err(schema(
                        "ipc-depth",
                        name,
                        "message nesting exceeds 16",
                        "flatten the application message graph",
                    ));
                }
                done.insert(name, depth);
                continue;
            }
            if !visiting.insert(name) {
                return Err(schema(
                    "ipc-recursion",
                    name,
                    "cyclic message graph",
                    "use acyclic messages with depth at most 16",
                ));
            }
            stack.push((name, true));
            for child in message
                .fields
                .iter()
                .filter(|f| f.kind == "message")
                .filter_map(|f| f.target.as_deref())
            {
                stack.push((child, false));
            }
        }
    }
    Ok(())
}
