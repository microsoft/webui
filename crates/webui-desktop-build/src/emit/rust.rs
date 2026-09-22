// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::BTreeSet;
use std::fmt::Write;

use super::{message, short, HEADER};
use crate::{
    model::{Contract, Method},
    names::{pascal, rust_ident, snake},
    GenerateError,
};

pub(crate) fn emit(contract: &Contract, hash: &str) -> Result<String, GenerateError> {
    let mut out = String::from(HEADER);
    out.push_str(
        "pub mod messages { include!(\"ipc_messages.rs\"); }\nuse webui_desktop::ipc::*;\n",
    );
    writeln!(out, "pub const SCHEMA_HASH: &str = {hash:?};").ok();
    out.push_str("pub static MESSAGE_SHAPES: &[MessageShape] = &[\n");
    for msg in &contract.messages {
        out.push_str("MessageShape { fields: &[\n");
        let oneofs: BTreeSet<_> = msg
            .fields
            .iter()
            .filter_map(|f| f.oneof.as_deref())
            .collect();
        for field in &msg.fields {
            let kind = if field.kind == "message" {
                let (index, _) = message(contract, field.target.as_deref().unwrap_or(""))?;
                format!("FieldKind::Message({index})")
            } else {
                format!("FieldKind::{}", pascal(&field.kind))
            };
            let oneof = field
                .oneof
                .as_deref()
                .and_then(|name| oneofs.iter().position(|n| *n == name));
            writeln!(out, "FieldShape {{ number: {}, kind: {kind}, repeated: {}, oneof: {oneof:?}, map_key: {} }},", field.number, field.repeated, field.map_key).ok();
        }
        out.push_str("] },\n");
    }
    out.push_str("];\n");
    for (i, _) in contract.messages.iter().enumerate() {
        writeln!(out, "fn validate_{i}(bytes: &[u8], limits: &IpcLimits) -> Result<(), IpcError> {{ validate_message(bytes, {i}, MESSAGE_SHAPES, limits) }}").ok();
    }
    writeln!(out, "pub static SCHEMA: IpcSchema = IpcSchema {{ name: {:?}, major: {}, hash: SCHEMA_HASH, methods: &[", contract.name, contract.major).ok();
    for method in &contract.methods {
        let (input, _) = message(contract, &method.input)?;
        let response = if method.kind == "rpc" {
            format!("Some(validate_{})", message(contract, &method.output)?.0)
        } else {
            "None".into()
        };
        writeln!(out, "MethodDescriptor {{ id: {}, name: {:?}, receiver: Endpoint::{}, kind: MethodKind::{}, development_only: {}, validate_request: validate_{input}, validate_response: {response} }},", method.id, method.name, pascal(&method.receiver), pascal(&method.kind), method.development_only).ok();
    }
    out.push_str("] };\n");
    let services: BTreeSet<_> = contract
        .methods
        .iter()
        .map(|m| m.service.as_str())
        .collect();
    for service in services {
        let methods: Vec<_> = contract
            .methods
            .iter()
            .filter(|m| m.service == service)
            .collect();
        emit_service(&mut out, contract, service, &methods)?;
    }
    Ok(out)
}

fn emit_service(
    out: &mut String,
    contract: &Contract,
    service: &str,
    methods: &[&Method],
) -> Result<(), GenerateError> {
    let module = rust_ident(&snake(service));
    writeln!(out, "pub mod {module} {{ use super::*;").ok();
    for method in methods {
        let marker = rust_ident(&pascal(short(&method.name)));
        let request = &message(contract, &method.input)?.1.rust;
        let receiver = pascal(&method.receiver);
        writeln!(out, "#[derive(Debug, Clone, Copy)] pub struct {marker};").ok();
        if method.kind == "rpc" {
            let response = &message(contract, &method.output)?.1.rust;
            writeln!(out, "impl Rpc for {marker} {{ type Request = {request}; type Response = {response}; type Receiver = {receiver}; const ID: u32 = {}; }}", method.id).ok();
        } else {
            writeln!(out, "impl Event for {marker} {{ type Payload = {request}; type Receiver = {receiver}; const ID: u32 = {}; }}", method.id).ok();
            if method.receiver == "host" {
                let name = snake(short(&method.name));
                writeln!(out, "pub fn subscribe_{name}<F, Fut>(session: &IpcSession, callback: F) -> Result<IpcSubscription, IpcError> where F: Fn(NotificationContext, {request}) -> Fut + Send + Sync + 'static, Fut: std::future::Future<Output = Result<(), IpcError>> + Send + 'static {{ session.subscribe::<{marker}, F, Fut>(callback) }}").ok();
            }
        }
    }
    out.push_str("}\n");
    if methods.first().is_some_and(|m| m.receiver == "host") {
        emit_host(out, contract, service, methods)
    } else {
        emit_client(out, contract, service, methods)
    }
}

fn emit_host(
    out: &mut String,
    contract: &Contract,
    service: &str,
    methods: &[&Method],
) -> Result<(), GenerateError> {
    let handler = format!("{}Handler", pascal(service));
    writeln!(out, "pub trait {handler}: Send + Sync + 'static {{").ok();
    for method in methods {
        let name = rust_ident(&snake(short(&method.name)));
        let request = &message(contract, &method.input)?.1.rust;
        let (context, response) = if method.kind == "rpc" {
            (
                "RequestContext",
                message(contract, &method.output)?.1.rust.as_str(),
            )
        } else {
            ("NotificationContext", "()")
        };
        writeln!(
            out,
            "fn {name}(&self, context: {context}, request: {request}) -> IpcFuture<{response}>;"
        )
        .ok();
    }
    out.push_str("}\n");
    writeln!(out, "pub fn register_{}(registry: &mut IpcRegistry, handler: std::sync::Arc<dyn {handler}>) -> Result<(), IpcError> {{", snake(service)).ok();
    out.push_str("registry.validate_registration(&[");
    for method in methods {
        write!(out, "{},", method.id).ok();
    }
    out.push_str("])?;\n");
    for method in methods {
        let name = rust_ident(&snake(short(&method.name)));
        let register = if method.kind == "rpc" {
            "register"
        } else {
            "register_notification"
        };
        writeln!(out, "{{ let handler = std::sync::Arc::clone(&handler); registry.{register}::<{}::{}, _, _>(move |context, request| handler.{name}(context, request))?; }}", rust_ident(&snake(service)), rust_ident(&pascal(short(&method.name)))).ok();
    }
    out.push_str("Ok(()) }\n");
    Ok(())
}

fn emit_client(
    out: &mut String,
    contract: &Contract,
    service: &str,
    methods: &[&Method],
) -> Result<(), GenerateError> {
    let client = format!("{}Client", pascal(service));
    writeln!(out, "#[derive(Clone)] pub struct {client} {{ session: IpcSession }}\nimpl {client} {{ pub fn new(session: IpcSession) -> Self {{ Self {{ session }} }}").ok();
    for method in methods {
        let name = rust_ident(&snake(short(&method.name)));
        let marker = format!(
            "{}::{}",
            rust_ident(&snake(service)),
            rust_ident(&pascal(short(&method.name)))
        );
        let request = &message(contract, &method.input)?.1.rust;
        if method.kind == "rpc" {
            let response = &message(contract, &method.output)?.1.rust;
            writeln!(out, "pub fn {name}(&self, request: {request}, options: CallOptions) -> IpcCall<{response}> {{ self.session.call::<{marker}>(request, options) }}").ok();
        } else {
            writeln!(out, "pub fn {name}(&self, payload: {request}) -> IpcFuture<()> {{ self.session.notify::<{marker}>(payload) }}").ok();
        }
    }
    out.push_str("}\n");
    Ok(())
}
