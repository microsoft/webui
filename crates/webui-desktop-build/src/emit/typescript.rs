// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
};

use super::{message, short, HEADER};
use crate::{
    error::schema,
    model::{Contract, Message, Method, EMPTY},
    names::{camel, ts_field},
    GenerateError,
};

const CONNECT_SIGNATURE: &str = "export async function connectDesktop(transport: IpcTransport, options?: { renderer?: RendererHandlers; onError?: (error: IpcError) => void }): Promise<AppConnection>";

pub(crate) fn emit(contract: &Contract, hash: &str) -> Result<(String, String), GenerateError> {
    let files: BTreeSet<_> = contract
        .messages
        .iter()
        .filter(|m| !m.map_entry)
        .map(|m| m.file.as_str())
        .collect();
    let files: BTreeMap<_, _> = files.into_iter().enumerate().map(|(i, f)| (f, i)).collect();
    let facade = emit_facade(contract, hash, &files)?;
    let mut out = String::from(HEADER);
    out.push_str("// Private implementation: application code imports ./ipc.js instead.\n");
    out.push_str("import type { AppConnection, RendererHandlers } from './ipc.js';\n");
    out.push_str("import { connect, validateMessage, validateValue, IpcError, type MessageShape, type MessageCodec, type ConnectionSchema, type RuntimeConnection, type IpcTransport, type RequestContext } from '@microsoft/webui-desktop';\n");
    for (file, i) in &files {
        let base = file.strip_suffix(".proto").unwrap_or(file);
        writeln!(
            out,
            "import * as codec_{i} from {:?};",
            format!("./{base}.js")
        )
        .ok();
    }
    writeln!(out, "export const schemaHash = {hash:?};").ok();
    emit_shapes(&mut out, contract)?;
    for (i, msg) in contract
        .messages
        .iter()
        .enumerate()
        .filter(|(_, m)| !m.map_entry)
    {
        let codec = format!("codec_{}.{}", files[msg.file.as_str()], msg.ts);
        let ty = ts_type(msg, &files);
        writeln!(out, "const message_{i}: MessageCodec<{ty}> = {{").ok();
        if msg.name == EMPTY {
            writeln!(out, "encode: () => {codec}.encode({{}}).finish(), decode: bytes => {{ {codec}.decode(bytes); }},\nvalidate: value => {{ if (value !== undefined) throw new IpcError('invalid-payload', 'Empty requires undefined'); }},").ok();
        } else {
            writeln!(out, "encode: value => {codec}.encode(value).finish(), decode: bytes => {codec}.decode(bytes),\nvalidate: (value, limits) => validateValue(value, {i}, messageShapes, limits),").ok();
        }
        writeln!(out, "validateBytes: (bytes, limits) => validateMessage(bytes, {i}, messageShapes, limits),\n}};").ok();
    }
    // `wireVersion` must match `webui_desktop::ipc::IPC_VERSION` (currently 3):
    // the generated renderer and the framework's Rust host ship in the same
    // binary and are never independently versioned.
    writeln!(out, "export const schema: ConnectionSchema = {{ hello: {{ wireVersion: 3, contractName: {:?}, contractMajor: {}, schemaHash }}, methods: [", contract.name, contract.major).ok();
    for method in &contract.methods {
        let response = if method.kind == "rpc" {
            format!(
                ", response: message_{}",
                message(contract, &method.output)?.0
            )
        } else {
            String::new()
        };
        writeln!(out, "{{ id: {}, name: {:?}, receiver: {:?}, kind: {:?}, developmentOnly: {}, request: message_{}{response} }},", method.id, method.name, method.receiver, method.kind, method.development_only, message(contract, &method.input)?.0).ok();
    }
    out.push_str("] };\n");
    emit_connection(&mut out, contract)?;
    Ok((facade, out))
}

fn emit_facade(
    contract: &Contract,
    hash: &str,
    files: &BTreeMap<&str, usize>,
) -> Result<String, GenerateError> {
    let mut out = String::from(HEADER);
    out.push_str("import type { IpcError, IpcTransport, CallOptions, RequestContext, Subscription, DesktopConnection } from '@microsoft/webui-desktop';\n");
    for (file, i) in files {
        let base = file.strip_suffix(".proto").unwrap_or(file);
        writeln!(
            out,
            "import type * as codec_{i} from {:?};",
            format!("./{base}.js")
        )
        .ok();
    }
    writeln!(out, "export const schemaHash = {hash:?};").ok();
    emit_interfaces(&mut out, contract, files)?;
    out.push_str("let runtime: Promise<typeof import('./ipc-runtime.js')> | undefined;\n");
    writeln!(out, "{CONNECT_SIGNATURE} {{").ok();
    out.push_str("const implementation = await (runtime ??= import('./ipc-runtime.js'));\nreturn implementation.connectDesktop(transport, options);\n}\n");
    Ok(out)
}

fn ts_type(msg: &Message, files: &BTreeMap<&str, usize>) -> String {
    if msg.name == EMPTY {
        "void".into()
    } else {
        format!("codec_{}.{}", files[msg.file.as_str()], msg.ts)
    }
}

fn emit_shapes(out: &mut String, contract: &Contract) -> Result<(), GenerateError> {
    out.push_str("export const messageShapes: readonly MessageShape[] = [\n");
    for msg in &contract.messages {
        out.push_str("{ fields: [\n");
        for field in &msg.fields {
            let child = if field.kind == "message" {
                format!(
                    ", message: {}",
                    message(contract, field.target.as_deref().unwrap_or(""))?.0
                )
            } else {
                String::new()
            };
            let oneof = field
                .oneof
                .as_ref()
                .map(|n| format!(", oneof: {:?}", ts_field(n)))
                .unwrap_or_default();
            writeln!(out, "{{ number: {}, name: {:?}, kind: {:?}{child}, repeated: {}, optional: {}, mapKey: {}, map: {}{oneof} }},", field.number, ts_field(&field.name), field.kind, field.repeated, field.optional, field.map_key, field.map).ok();
        }
        out.push_str("] },\n");
    }
    out.push_str("];\n");
    Ok(())
}

fn signature(
    method: &Method,
    contract: &Contract,
    files: &BTreeMap<&str, usize>,
) -> Result<(String, String, String), GenerateError> {
    let name = camel(short(&method.name));
    let input = ts_type(message(contract, &method.input)?.1, files);
    let output = if method.kind == "rpc" {
        ts_type(message(contract, &method.output)?.1, files)
    } else {
        "void".into()
    };
    Ok((name, input, output))
}

fn emit_interfaces(
    out: &mut String,
    contract: &Contract,
    files: &BTreeMap<&str, usize>,
) -> Result<(), GenerateError> {
    let mut names = BTreeSet::new();
    for method in &contract.methods {
        if !names.insert((method.receiver.as_str(), camel(short(&method.name)))) {
            return Err(schema(
                "ipc-name-collision",
                &method.name,
                "flattened endpoint method names collide",
                "use distinct method names within each receiver role",
            ));
        }
    }
    out.push_str("export interface HostClient {\n");
    for method in contract.methods.iter().filter(|m| m.receiver == "host") {
        let (name, input, output) = signature(method, contract, files)?;
        let options = if method.kind == "rpc" {
            ", options?: CallOptions"
        } else {
            ""
        };
        writeln!(out, "{name}(request: {input}{options}): Promise<{output}>;").ok();
    }
    out.push_str("}\nexport interface RendererHandlers {\n");
    for method in contract
        .methods
        .iter()
        .filter(|m| m.receiver == "renderer" && m.kind == "rpc")
    {
        let (name, input, output) = signature(method, contract, files)?;
        writeln!(
            out,
            "{name}(request: {input}, context: RequestContext): Promise<{output}> | {output};"
        )
        .ok();
    }
    out.push_str("}\nexport interface RendererEvents {\n");
    for method in contract
        .methods
        .iter()
        .filter(|m| m.receiver == "renderer" && m.kind == "notification")
    {
        let (name, input, _) = signature(method, contract, files)?;
        writeln!(
            out,
            "{}(callback: (payload: {input}) => void | Promise<void>): Subscription;",
            event_name(&name)
        )
        .ok();
    }
    out.push_str("}\nexport interface AppConnection extends DesktopConnection { readonly host: HostClient; readonly renderer: RendererEvents; setRenderer(handlers: RendererHandlers): Subscription; }\n");
    Ok(())
}

fn event_name(name: &str) -> String {
    let mut chars = name.chars();
    let mut result = String::from("on");
    if let Some(c) = chars.next() {
        result.push(c.to_ascii_uppercase());
        result.extend(chars);
    }
    result
}

fn emit_connection(out: &mut String, contract: &Contract) -> Result<(), GenerateError> {
    out.push_str("function handlersMap(handlers: RendererHandlers): ReadonlyMap<number, (value: any, context: RequestContext) => any> {\nif (!handlers || typeof handlers !== 'object') throw new IpcError('invalid-payload', 'Renderer handlers must be an object');\n");
    for method in contract
        .methods
        .iter()
        .filter(|m| m.receiver == "renderer" && m.kind == "rpc")
    {
        let name = camel(short(&method.name));
        writeln!(out, "if (typeof handlers.{name} !== 'function') throw new IpcError('invalid-payload', 'Missing renderer handler: {name}');").ok();
    }
    out.push_str("return new Map<number, (value: any, context: RequestContext) => any>([\n");
    for method in contract
        .methods
        .iter()
        .filter(|m| m.receiver == "renderer" && m.kind == "rpc")
    {
        writeln!(
            out,
            "[{}, (value, context) => handlers.{}(value, context)],",
            method.id,
            camel(short(&method.name))
        )
        .ok();
    }
    out.push_str("]);\n}\n");
    writeln!(out, "{CONNECT_SIGNATURE} {{").ok();
    out.push_str("const connection: RuntimeConnection = await connect(transport, schema, { ...(options?.renderer ? { handlers: handlersMap(options.renderer) } : {}), ...(options?.onError ? { onError: options.onError } : {}) });\nreturn { close: () => connection.close(), closed: connection.closed, setRenderer: handlers => connection.register(handlersMap(handlers)), host: {\n");
    for method in contract.methods.iter().filter(|m| m.receiver == "host") {
        let name = camel(short(&method.name));
        if method.kind == "rpc" {
            writeln!(
                out,
                "{name}: (request, options) => connection.call({}, request, options),",
                method.id
            )
            .ok();
        } else {
            writeln!(
                out,
                "{name}: request => connection.notify({}, request),",
                method.id
            )
            .ok();
        }
    }
    out.push_str("}, renderer: {\n");
    for method in contract
        .methods
        .iter()
        .filter(|m| m.receiver == "renderer" && m.kind == "notification")
    {
        writeln!(
            out,
            "{}: callback => connection.subscribe({}, callback),",
            event_name(&camel(short(&method.name))),
            method.id
        )
        .ok();
    }
    out.push_str("} };\n}\n");
    Ok(())
}
