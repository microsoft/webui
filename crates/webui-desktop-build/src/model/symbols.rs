// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{Contract, Method};
use crate::{error::schema, names, GenerateError};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn validate(contract: &Contract) -> Result<(), GenerateError> {
    let mut methods = BTreeMap::<&str, BTreeSet<String>>::new();
    let mut markers = BTreeMap::<&str, BTreeSet<String>>::new();
    let mut endpoint_methods = BTreeSet::new();
    for method in &contract.methods {
        let service = names::rust_ident(&names::snake(&method.service));
        if !identifier(&service) {
            return Err(collision(
                method,
                &service,
                "an invalid normalized Rust service identifier",
            ));
        }
        if service == "messages" {
            return Err(collision(
                method,
                "messages",
                "the generated Rust message module",
            ));
        }
        let short = method.name.rsplit('.').next().unwrap_or(&method.name);
        let rust_method = names::rust_ident(&names::snake(short));
        let marker = names::rust_ident(&names::pascal(short));
        let ts_method = names::camel(short);
        if !identifier(&rust_method) || !identifier(&marker) || !identifier(&ts_method) {
            return Err(collision(
                method,
                short,
                "an invalid normalized method or marker identifier",
            ));
        }
        if method.receiver == "renderer" && rust_method == "new" {
            return Err(collision(
                method,
                "new",
                "the renderer client's session constructor",
            ));
        }
        if ts_method == "constructor" {
            return Err(collision(
                method,
                &ts_method,
                "JavaScript's inherited constructor property",
            ));
        }
        // These names are referenced unqualified in the emitted service module.
        if matches!(
            marker.as_str(),
            "Rpc"
                | "Event"
                | "Host"
                | "Renderer"
                | "IpcSession"
                | "IpcSubscription"
                | "IpcError"
                | "NotificationContext"
                | "Send"
                | "Sync"
                | "Fn"
                | "Result"
                | "F"
                | "Fut"
        ) {
            return Err(collision(
                method,
                &marker,
                "a generated marker's runtime type or generic parameter",
            ));
        }
        if !methods
            .entry(&method.service)
            .or_default()
            .insert(rust_method.clone())
        {
            return Err(collision(
                method,
                &rust_method,
                "another normalized Rust method",
            ));
        }
        if !markers
            .entry(&method.service)
            .or_default()
            .insert(marker.clone())
        {
            return Err(collision(method, &marker, "another normalized Rust marker"));
        }
        if !endpoint_methods.insert((&method.receiver, ts_method.clone())) {
            return Err(collision(
                method,
                &ts_method,
                "another normalized TypeScript endpoint method",
            ));
        }
    }
    Ok(())
}

fn identifier(name: &str) -> bool {
    let name = name.strip_prefix("r#").unwrap_or(name);
    name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') && name != "_"
}

fn collision(method: &Method, symbol: &str, owner: &str) -> GenerateError {
    schema("ipc-name-collision", &method.name,
        format!("generated symbol {symbol:?} conflicts with {owner}"),
        "rename the schema method or service so its normalized generated name is distinct; renderer method names must not normalize to new")
}
