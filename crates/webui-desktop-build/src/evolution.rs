// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{error::schema, model::Contract, GenerateError};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Lock {
    pub format: u32,
    pub schema_hash: String,
    pub contract: Contract,
    pub retired_ids: BTreeSet<u32>,
}

pub(crate) fn update(
    contract: &Contract,
    hash: &str,
    previous: Option<&[u8]>,
) -> Result<Lock, GenerateError> {
    let mut retired_ids = BTreeSet::new();
    let mut history = contract.clone();
    if let Some(bytes) = previous {
        let old: Lock = serde_json::from_slice(bytes).map_err(|e| {
            schema(
                "ipc-lock",
                "compatibility lock",
                e.to_string(),
                "restore a valid checked-in lock; do not discard compatibility history",
            )
        })?;
        if old.format != 1
            || old.contract.name != contract.name
            || contract.major < old.contract.major
        {
            return Err(schema(
                "ipc-lock-contract",
                &contract.name,
                "lock format, contract name, or major is incompatible",
                "retain the contract identity and use a nondecreasing major version",
            ));
        }
        retired_ids = old.retired_ids;
        for method in &contract.methods {
            if old
                .contract
                .methods
                .iter()
                .any(|m| m.name == method.name && m.id != method.id)
            {
                return Err(schema(
                    "ipc-method-id-change",
                    &method.name,
                    "existing method ID changed",
                    "retain the original ID; introduce a newly named method for a new identity",
                ));
            }
            if retired_ids.contains(&method.id) {
                return Err(schema(
                    "ipc-retired-id",
                    &method.name,
                    "method ID has been retired",
                    "assign a new ID; retired IDs must never be reused",
                ));
            }
            if let Some(previous) = old.contract.methods.iter().find(|m| m.id == method.id) {
                if previous.name != method.name {
                    return Err(schema("ipc-method-rename", &method.name, "method ID belongs to a different name", "for an intentional rename, explicitly update the checked-in lock name while retaining its ID"));
                }
                if (previous.receiver != method.receiver
                    || previous.kind != method.kind
                    || previous.input != method.input
                    || previous.output != method.output
                    || previous.development_only != method.development_only)
                    && contract.major == old.contract.major
                {
                    return Err(schema(
                        "ipc-breaking-method",
                        &method.name,
                        "method contract changed without a major version increment",
                        "increment contract_major and deploy both endpoints together",
                    ));
                }
            }
        }
        for method in &old.contract.methods {
            if !contract.methods.iter().any(|m| m.id == method.id) {
                retired_ids.insert(method.id);
            }
        }
        check_messages(contract, &old.contract)?;
        check_enums(contract, &old.contract)?;
        // Keep disconnected type history: removing all uses must not allow a
        // later reintroduction to silently reuse retired fields or enum values.
        for message in old.contract.messages {
            if !history.messages.iter().any(|m| m.name == message.name) {
                history.messages.push(message);
            }
        }
        for enumeration in old.contract.enums {
            if !history.enums.iter().any(|e| e.name == enumeration.name) {
                history.enums.push(enumeration);
            }
        }
    }
    history.messages.sort_by(|a, b| a.name.cmp(&b.name));
    history.enums.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Lock {
        format: 1,
        schema_hash: hash.into(),
        contract: history,
        retired_ids,
    })
}

fn check_messages(new: &Contract, old: &Contract) -> Result<(), GenerateError> {
    for previous in &old.messages {
        let Some(message) = new.messages.iter().find(|m| m.name == previous.name) else {
            continue;
        };
        for field in &previous.fields {
            if let Some(current) = message.fields.iter().find(|f| f.number == field.number) {
                if current.kind != field.kind
                    || current.target != field.target
                    || current.repeated != field.repeated
                    || current.map != field.map
                {
                    return Err(schema("ipc-field-type", &message.name, format!("field {} changes its protobuf type", field.number), "never change an existing field type; reserve the old number and add a new field"));
                }
                if (current.optional != field.optional
                    || current.oneof != field.oneof
                    || current.name != field.name)
                    && new.major == old.major
                {
                    return Err(schema(
                        "ipc-breaking-field",
                        &message.name,
                        format!("field {} changes its application API", field.number),
                        "increment contract_major for presence, oneof, or naming changes",
                    ));
                }
            } else {
                let reserved = message.reserved.iter().any(|(start, end)| {
                    i64::from(*start) <= i64::from(field.number)
                        && i64::from(field.number) < i64::from(*end)
                });
                if !reserved || !message.reserved_names.contains(&field.name) {
                    return Err(schema(
                        "ipc-field-retirement",
                        &message.name,
                        format!(
                            "removed field {} is not reserved by number and name",
                            field.number
                        ),
                        "add reserved declarations for the removed field number and name",
                    ));
                }
            }
        }
        for field in &message.fields {
            if previous.reserved.iter().any(|(a, b)| {
                i64::from(*a) <= i64::from(field.number) && i64::from(field.number) < i64::from(*b)
            }) || previous.reserved_names.contains(&field.name)
            {
                return Err(schema(
                    "ipc-retired-field",
                    &message.name,
                    "reserved field number or name was reused",
                    "choose a fresh field number and name",
                ));
            }
        }
        // Reservation history cannot be erased and then reused in a later generation.
        for (a, b) in &previous.reserved {
            if !message.reserved.iter().any(|(c, d)| c <= a && d >= b) {
                return Err(schema(
                    "ipc-reservation-history",
                    &message.name,
                    "field reservation was removed",
                    "retain all reserved field ranges and names",
                ));
            }
        }
        if previous
            .reserved_names
            .iter()
            .any(|n| !message.reserved_names.contains(n))
        {
            return Err(schema(
                "ipc-reservation-history",
                &message.name,
                "field name reservation was removed",
                "retain all reserved names",
            ));
        }
    }
    Ok(())
}

fn check_enums(new: &Contract, old: &Contract) -> Result<(), GenerateError> {
    for previous in &old.enums {
        let Some(current) = new.enums.iter().find(|e| e.name == previous.name) else {
            continue;
        };
        for (name, number) in &previous.values {
            match current.values.get(name) {
                Some(value) if value == number => {}
                Some(_) => {
                    return Err(schema(
                        "ipc-enum-number",
                        &current.name,
                        "enum value changed number",
                        "retain the original numeric value",
                    ))
                }
                None => {
                    if !current.reserved_names.contains(name)
                        || !current
                            .reserved
                            .iter()
                            .any(|(a, b)| a <= number && number <= b)
                    {
                        return Err(schema(
                            "ipc-enum-retirement",
                            &current.name,
                            "removed enum value is not reserved",
                            "reserve the removed enum name and number",
                        ));
                    }
                }
            }
        }
        for (name, number) in &current.values {
            if previous.reserved_names.contains(name)
                || previous
                    .reserved
                    .iter()
                    .any(|(a, b)| a <= number && number <= b)
            {
                return Err(schema(
                    "ipc-retired-enum",
                    &current.name,
                    "retired enum value was reused",
                    "use a fresh enum name and number",
                ));
            }
        }
        if previous
            .reserved_names
            .iter()
            .any(|n| !current.reserved_names.contains(n))
            || previous
                .reserved
                .iter()
                .any(|(a, b)| !current.reserved.iter().any(|(c, d)| c <= a && b <= d))
        {
            return Err(schema(
                "ipc-reservation-history",
                &current.name,
                "enum reservation was removed",
                "retain all reserved enum numbers and names",
            ));
        }
    }
    Ok(())
}
