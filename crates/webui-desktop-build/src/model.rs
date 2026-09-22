// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::{BTreeMap, BTreeSet};

use prost_reflect::{
    DescriptorPool, DynamicMessage, Kind, MessageDescriptor, ReflectMessage, Value,
};
use serde::{Deserialize, Serialize};

use crate::{error::schema, names, GenerateError};
mod symbols;
mod validation;
use validation::{validate_all_messages, validate_message_descriptor, validate_topology};

pub(crate) const EMPTY: &str = "google.protobuf.Empty";
pub(crate) const NOTIFICATION: &str = "webui.ipc.Notification";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct Contract {
    pub name: String,
    pub major: u32,
    pub methods: Vec<Method>,
    pub messages: Vec<Message>,
    pub enums: Vec<Enumeration>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct Method {
    pub id: u32,
    pub name: String,
    pub service: String,
    pub receiver: String,
    pub kind: String,
    pub development_only: bool,
    pub input: String,
    pub output: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct Message {
    pub name: String,
    pub file: String,
    pub rust: String,
    pub ts: String,
    pub map_entry: bool,
    pub fields: Vec<Field>,
    pub reserved: Vec<(i32, i32)>,
    pub reserved_names: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct Field {
    pub number: u32,
    pub name: String,
    pub kind: String,
    pub target: Option<String>,
    pub repeated: bool,
    #[serde(default)]
    pub packed: bool,
    pub optional: bool,
    pub oneof: Option<String>,
    pub map: bool,
    pub map_key: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct Enumeration {
    pub name: String,
    pub values: BTreeMap<String, i32>,
    pub reserved: Vec<(i32, i32)>,
    pub reserved_names: Vec<String>,
}

pub(crate) fn option(options: &DynamicMessage, name: &str) -> Result<Value, GenerateError> {
    let ext = options
        .descriptor()
        .parent_pool()
        .get_extension_by_name(name)
        .ok_or_else(|| {
            schema(
                "ipc-options",
                name,
                "SDK option extension is missing",
                "import webui/ipc/options.proto from the SDK include directory",
            )
        })?;
    Ok(options.get_extension(&ext).into_owned())
}

fn allowed_options(options: DynamicMessage) -> Result<(), GenerateError> {
    const ALLOWED: &[&str] = &[
        "webui.ipc.contract_name",
        "webui.ipc.contract_major",
        "webui.ipc.receiver",
        "webui.ipc.id",
        "webui.ipc.notification",
        "webui.ipc.development_only",
    ];
    for (ext, _) in options.extensions() {
        if !ALLOWED.contains(&ext.full_name()) {
            return Err(schema(
                "ipc-unsupported-option",
                ext.full_name(),
                "custom application option is unsupported",
                "remove the custom option; only SDK IPC extensions are supported",
            ));
        }
    }
    Ok(())
}

pub(crate) fn application_file(name: &str) -> bool {
    !name.starts_with("google/protobuf/") && name != "webui/ipc/options.proto"
}

pub(crate) fn build(pool: &DescriptorPool) -> Result<Contract, GenerateError> {
    validate_all_messages(pool)?;
    let mut identity = None;
    for file in pool.files().filter(|f| application_file(f.name())) {
        if file.syntax() != prost_reflect::Syntax::Proto3 {
            return Err(schema(
                "ipc-proto3",
                file.name(),
                "application schemas must use proto3",
                "change the schema syntax to proto3",
            ));
        }
        if file.extensions().len() != 0 {
            return Err(schema(
                "ipc-extension",
                file.name(),
                "application extensions are unsupported",
                "use ordinary proto3 fields",
            ));
        }
        allowed_options(file.options())?;
        if let (Value::String(name), Value::U32(major)) = (
            option(&file.options(), "webui.ipc.contract_name")?,
            option(&file.options(), "webui.ipc.contract_major")?,
        ) {
            if name.is_empty() && major == 0 {
                continue;
            }
            if name.is_empty() || major == 0 || name.len() > 256 {
                return Err(schema("ipc-contract", file.name(), "contract name and positive major are required together", "set contract_name (at most 256 bytes) and contract_major on the application root"));
            }
            let current = (name, major);
            if identity.as_ref().is_some_and(|old| old != &current) {
                return Err(schema(
                    "ipc-contract",
                    file.name(),
                    "roots declare different contracts",
                    "generate one contract at a time",
                ));
            }
            identity = Some(current);
        }
    }
    let (name, major) = identity.ok_or_else(|| {
        schema(
            "ipc-contract",
            "application roots",
            "missing contract identity",
            "set (webui.ipc.contract_name) and (webui.ipc.contract_major)",
        )
    })?;
    let methods = methods(pool)?;
    let mut reachable = BTreeSet::new();
    let mut pending = Vec::new();
    for method in &methods {
        pending.push(method.input.clone());
        if method.kind == "rpc" {
            pending.push(method.output.clone());
        }
    }
    let mut messages = BTreeMap::new();
    let mut enums: BTreeMap<String, Enumeration> = BTreeMap::new();
    while let Some(name) = pending.pop() {
        if !reachable.insert(name.clone()) {
            continue;
        }
        if reachable.len() > 4096 {
            return Err(schema(
                "ipc-schema-limit",
                name,
                "more than 4096 reachable messages",
                "split the application contract",
            ));
        }
        let desc = pool.get_message_by_name(&name).ok_or_else(|| {
            schema(
                "ipc-message",
                &name,
                "message descriptor missing",
                "check imported schema definitions",
            )
        })?;
        validate_message_descriptor(&desc)?;
        for field in desc.fields() {
            allowed_options(field.options())?;
            match field.kind() {
                Kind::Message(m) => pending.push(m.full_name().into()),
                Kind::Enum(e) => {
                    allowed_options(e.options())?;
                    let raw = e.enum_descriptor_proto();
                    let mut reserved: Vec<_> = raw
                        .reserved_range
                        .iter()
                        .map(|r| (r.start(), r.end()))
                        .collect();
                    reserved.sort();
                    let mut reserved_names = raw.reserved_name.clone();
                    reserved_names.sort();
                    enums.insert(
                        e.full_name().into(),
                        Enumeration {
                            name: e.full_name().into(),
                            values: e.values().map(|v| (v.name().into(), v.number())).collect(),
                            reserved,
                            reserved_names,
                        },
                    );
                }
                _ => {}
            }
        }
        messages.insert(name, message(&desc));
    }
    let contract = Contract {
        name,
        major,
        methods,
        messages: messages.into_values().collect(),
        enums: enums.into_values().collect(),
    };
    symbols::validate(&contract)?;
    validate_topology(&contract)?;
    Ok(contract)
}

fn methods(pool: &DescriptorPool) -> Result<Vec<Method>, GenerateError> {
    let mut result = BTreeMap::new();
    let mut service_names = BTreeSet::new();
    for service in pool.services() {
        allowed_options(service.options())?;
        if !service_names.insert(names::snake(service.name())) {
            return Err(schema(
                "ipc-name-collision",
                service.full_name(),
                "generated service names collide",
                "use distinct service names across packages",
            ));
        }
        let receiver = match option(&service.options(), "webui.ipc.receiver")? {
            Value::EnumNumber(1) => "host",
            Value::EnumNumber(2) => "renderer",
            _ => {
                return Err(schema(
                    "ipc-receiver",
                    service.full_name(),
                    "service receiver is missing or invalid",
                    "set (webui.ipc.receiver) to HOST or RENDERER",
                ))
            }
        };
        let mut method_names = BTreeSet::new();
        for method in service.methods() {
            allowed_options(method.options())?;
            if !method_names.insert(names::snake(method.name())) {
                return Err(schema(
                    "ipc-name-collision",
                    method.full_name(),
                    "generated method names collide",
                    "choose distinct method names",
                ));
            }
            if method.is_client_streaming() || method.is_server_streaming() {
                return Err(schema(
                    "ipc-streaming",
                    method.full_name(),
                    "streaming IPC is unsupported",
                    "use a unary RPC or notification",
                ));
            }
            let id = match option(&method.options(), "webui.ipc.id")? {
                Value::U32(id) if id > 1023 => id,
                _ => {
                    return Err(schema(
                        "ipc-method-id",
                        method.full_name(),
                        "method ID is missing or reserved",
                        "assign an explicit unique ID in 1024..4294967295",
                    ))
                }
            };
            let notification =
                option(&method.options(), "webui.ipc.notification")? == Value::Bool(true);
            if notification != (method.output().full_name() == NOTIFICATION)
                || method.input().full_name() == NOTIFICATION
            {
                return Err(schema("ipc-notification", method.full_name(), "notification option and response marker must be used together", "use notification=true with webui.ipc.Notification; use google.protobuf.Empty for acknowledged completion"));
            }
            let value = Method {
                id,
                name: method.full_name().into(),
                service: service.name().into(),
                receiver: receiver.into(),
                kind: if notification { "notification" } else { "rpc" }.into(),
                development_only: option(&method.options(), "webui.ipc.development_only")?
                    == Value::Bool(true),
                input: method.input().full_name().into(),
                output: method.output().full_name().into(),
            };
            if result.insert(id, value).is_some() {
                return Err(schema(
                    "ipc-duplicate-id",
                    method.full_name(),
                    "ID already belongs to another method",
                    "assign a globally unique method ID",
                ));
            }
        }
    }
    if result.is_empty() {
        return Err(schema(
            "ipc-no-methods",
            "application",
            "contract has no methods",
            "declare at least one IPC service method",
        ));
    }
    Ok(result.into_values().collect())
}

fn message(desc: &MessageDescriptor) -> Message {
    let mut ancestry = Vec::new();
    let mut current = desc.parent_message();
    while let Some(parent) = current {
        ancestry.push(parent.name().to_owned());
        current = parent.parent_message();
    }
    ancestry.reverse();
    let mut rust = String::from("messages");
    for part in desc.package_name().split('.').filter(|p| !p.is_empty()) {
        rust.push_str("::");
        rust.push_str(&names::rust_ident(&names::snake(part)));
    }
    for part in &ancestry {
        rust.push_str("::");
        rust.push_str(&names::rust_ident(&names::snake(part)));
    }
    rust.push_str("::");
    rust.push_str(&names::rust_ident(&names::pascal(desc.name())));
    ancestry.push(desc.name().into());
    let mut fields: Vec<_> = desc
        .fields()
        .map(|field| {
            let (kind, target) = match field.kind() {
                Kind::Message(m) => ("message".into(), Some(m.full_name().into())),
                Kind::Enum(e) => ("enum".into(), Some(e.full_name().into())),
                other => (format!("{other:?}").to_lowercase(), None),
            };
            Field {
                number: field.number(),
                name: field.name().into(),
                kind,
                target,
                repeated: field.is_list() || field.is_map(),
                packed: field.is_packed(),
                optional: field.supports_presence(),
                oneof: field
                    .containing_oneof()
                    .filter(|_| !field.field_descriptor_proto().proto3_optional())
                    .map(|o| o.name().into()),
                map: field.is_map(),
                map_key: desc.is_map_entry() && field.number() == 1,
            }
        })
        .collect();
    fields.sort_by_key(|f| f.number);
    let raw = desc.descriptor_proto();
    let mut reserved: Vec<_> = raw
        .reserved_range
        .iter()
        .map(|r| (r.start(), r.end()))
        .collect();
    reserved.sort();
    let mut reserved_names = raw.reserved_name.clone();
    reserved_names.sort();
    Message {
        name: desc.full_name().into(),
        file: desc.parent_file().name().into(),
        rust: if desc.full_name() == EMPTY {
            "()".into()
        } else {
            rust
        },
        ts: ancestry.join("_"),
        map_entry: desc.is_map_entry(),
        fields,
        reserved,
        reserved_names,
    }
}
