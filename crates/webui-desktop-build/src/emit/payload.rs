// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
    path::{Path, PathBuf},
};

use crate::{
    emit::HEADER,
    model::{Contract, Enumeration, Field, Message, EMPTY},
    names::{camel, pascal, rust_ident, snake, ts_field},
};

pub(crate) fn rust(contract: &Contract, out_dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let packages = packages(contract);
    let mut include = String::from(HEADER);
    emit_rust_package_tree(&mut include, &packages, "");
    files.insert(out_dir.join("ipc_messages.rs"), include.into_bytes());
    for package in packages {
        if package.is_empty() {
            continue;
        }
        let top_level = top_level_messages(contract, &package);
        let mut out = String::from(HEADER);
        if top_level
            .iter()
            .any(|message| message_uses_maps(contract, message))
        {
            out.push_str("use std::collections::BTreeMap;\n");
        }
        out.push_str("use webui_desktop::ipc::{IpcCodec, IpcError, PayloadWriter, WireReader};\n");
        for message in top_level {
            emit_rust_message(&mut out, contract, message, &package);
        }
        for enumeration in contract
            .enums
            .iter()
            .filter(|e| package_of(&e.name, contract).as_deref() == Some(package.as_str()))
        {
            emit_rust_enum(&mut out, &enumeration.name, &enumeration.values);
        }
        files.insert(out_dir.join(format!("{package}.rs")), out.into_bytes());
    }
    files
}

pub(crate) fn typescript(contract: &Contract, out_dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut by_file: BTreeMap<&str, Vec<&Message>> = BTreeMap::new();
    for message in contract.messages.iter().filter(|m| m.name != EMPTY) {
        by_file.entry(&message.file).or_default().push(message);
    }
    for (file, messages) in by_file {
        let mut out = String::from(HEADER);
        out.push_str("import { BoundaryReader, PayloadWriter, readDelimited, readUint32, readUint64, skipField, IpcError } from '@microsoft/webui-desktop';\n");
        for import in ts_imports_for_file(contract, file, &messages) {
            writeln!(out, "{import}").ok();
        }
        for enumeration in ts_enums_for_file(contract, &messages) {
            emit_ts_enum(&mut out, &enumeration.name, &enumeration.values);
        }
        for message in messages.iter().filter(|m| !m.map_entry) {
            emit_ts_interface(&mut out, contract, message);
        }
        for message in messages.iter().filter(|m| !m.map_entry) {
            emit_ts_codec(&mut out, contract, message);
        }

        fn ts_imports_for_file(
            contract: &Contract,
            file: &str,
            messages: &[&Message],
        ) -> Vec<String> {
            let local = messages
                .iter()
                .map(|message| message.name.as_str())
                .collect::<BTreeSet<_>>();
            let mut imports = BTreeMap::<&str, BTreeSet<&str>>::new();
            for message in messages {
                for field in &message.fields {
                    if field.kind != "message" {
                        continue;
                    }
                    let Some(target) = field.target.as_deref() else {
                        continue;
                    };
                    if local.contains(target) || target == EMPTY {
                        continue;
                    }
                    let target_message = find_message(contract, target);
                    if target_message.file == file {
                        continue;
                    }
                    imports
                        .entry(target_message.file.as_str())
                        .or_default()
                        .insert(target_message.ts.as_str());
                }
            }
            imports
                .into_iter()
                .map(|(target_file, names)| {
                    let base = target_file.strip_suffix(".proto").unwrap_or(target_file);
                    let names = names.into_iter().collect::<Vec<_>>().join(", ");
                    format!("import {{ {names} }} from './{base}.js';")
                })
                .collect()
        }

        fn ts_enums_for_file<'a>(
            contract: &'a Contract,
            messages: &[&Message],
        ) -> Vec<&'a Enumeration> {
            let packages = messages
                .iter()
                .filter_map(|message| package_of(&message.name, contract))
                .collect::<BTreeSet<_>>();
            contract
                .enums
                .iter()
                .filter(|enumeration| {
                    package_of(&enumeration.name, contract)
                        .is_some_and(|package| packages.contains(&package))
                })
                .collect()
        }
        let base = file.strip_suffix(".proto").unwrap_or(file);
        files.insert(out_dir.join(format!("{base}.ts")), out.into_bytes());
    }
    files
}

fn packages(contract: &Contract) -> Vec<String> {
    let mut packages = BTreeSet::new();
    for message in contract.messages.iter().filter(|m| m.name != EMPTY) {
        if let Some(package) = package_of(&message.name, contract) {
            packages.insert(package);
        }
    }
    packages.into_iter().collect()
}

fn package_of(name: &str, contract: &Contract) -> Option<String> {
    for candidate in contract.messages.iter().filter(|m| m.name != EMPTY) {
        if candidate.name == name || name.starts_with(&(candidate.name.clone() + ".")) {
            let short = candidate.name.rsplit('.').next()?;
            return candidate
                .name
                .strip_suffix(short)
                .map(|p| p.trim_end_matches('.').to_owned());
        }
    }
    name.rsplit_once('.').map(|(package, _)| package.to_owned())
}

fn emit_rust_package_tree(out: &mut String, packages: &[String], prefix: &str) {
    let mut current = BTreeMap::<String, Vec<String>>::new();
    for package in packages {
        let remainder = if prefix.is_empty() {
            package.as_str()
        } else {
            package
                .strip_prefix(prefix)
                .and_then(|rest| rest.strip_prefix('.'))
                .unwrap_or("")
        };
        let mut parts = remainder.split('.');
        let Some(first) = parts.next() else { continue };
        if first.is_empty() {
            continue;
        }
        current
            .entry(first.to_owned())
            .or_default()
            .push(package.clone());
    }
    for (part, children) in current {
        let module = rust_ident(&snake(&part));
        writeln!(out, "pub mod {module} {{").ok();
        let package = if prefix.is_empty() {
            part.clone()
        } else {
            format!("{prefix}.{part}")
        };
        if children.iter().any(|child| child == &package) {
            writeln!(out, "include!(\"{package}.rs\");").ok();
        }
        let nested: Vec<String> = children.into_iter().filter(|c| c != &package).collect();
        if !nested.is_empty() {
            emit_rust_package_tree(out, &nested, &package);
        }
        out.push_str("}\n");
    }
}

fn top_level_messages<'a>(contract: &'a Contract, package: &str) -> Vec<&'a Message> {
    contract
        .messages
        .iter()
        .filter(|m| m.name != EMPTY && package_of(&m.name, contract).as_deref() == Some(package))
        .filter(|m| parent_message(&m.name, contract).is_none())
        .collect()
}

fn nested_messages<'a>(contract: &'a Contract, parent: &str) -> Vec<&'a Message> {
    contract
        .messages
        .iter()
        .filter(|m| parent_message(&m.name, contract).as_deref() == Some(parent))
        .collect()
}

fn parent_message(name: &str, contract: &Contract) -> Option<String> {
    contract
        .messages
        .iter()
        .filter(|m| name.starts_with(&(m.name.clone() + ".")))
        .map(|m| m.name.clone())
        .max_by_key(String::len)
}

fn emit_rust_message(out: &mut String, contract: &Contract, message: &Message, package: &str) {
    let name = rust_ident(&pascal(
        message.name.rsplit('.').next().unwrap_or(&message.name),
    ));
    if !message.map_entry {
        writeln!(
            out,
            "#[derive(Clone, Debug, Default, PartialEq)]\npub struct {name} {{"
        )
        .ok();
        for field in message.fields.iter().filter(|f| f.oneof.is_none()) {
            writeln!(
                out,
                "pub {}: {},",
                rust_ident(&snake(&field.name)),
                rust_field_type(contract, field, package)
            )
            .ok();
        }
        for oneof in oneofs(message) {
            writeln!(
                out,
                "pub {}: Option<{}::{}>,",
                rust_ident(&snake(&oneof)),
                snake(&name),
                pascal(&oneof)
            )
            .ok();
        }
        out.push_str("}\n");
    }
    for child in nested_messages(contract, &message.name) {
        if child.map_entry {
            emit_rust_message(out, contract, child, package);
        }
    }
    let nested = nested_messages(contract, &message.name)
        .into_iter()
        .filter(|m| !m.map_entry)
        .collect::<Vec<_>>();
    let oneofs = oneofs(message);
    if !nested.is_empty() || !oneofs.is_empty() {
        writeln!(
            out,
            "pub mod {} {{ #[allow(unused_imports)] use super::*;",
            snake(&name)
        )
        .ok();
        for oneof in oneofs {
            let oneof_ty = pascal(&oneof);
            writeln!(
                out,
                "#[derive(Clone, Debug, PartialEq)]\npub enum {oneof_ty} {{"
            )
            .ok();
            for field in message
                .fields
                .iter()
                .filter(|f| f.oneof.as_deref() == Some(&oneof))
            {
                writeln!(
                    out,
                    "{}({}),",
                    pascal(&field.name),
                    rust_scalar_type(contract, field, package)
                )
                .ok();
            }
            out.push_str("}\n");
        }
        for child in nested {
            emit_rust_message(out, contract, child, package);
        }
        out.push_str("}\n");
    }
    emit_rust_codec(out, contract, message, &name);
}

fn oneofs(message: &Message) -> Vec<String> {
    let mut set = BTreeSet::new();
    for field in &message.fields {
        if let Some(oneof) = &field.oneof {
            set.insert(oneof.clone());
        }
    }
    set.into_iter().collect()
}

fn message_uses_maps(contract: &Contract, message: &Message) -> bool {
    if message.fields.iter().any(|field| field.map) {
        return true;
    }
    nested_messages(contract, &message.name)
        .iter()
        .any(|child| message_uses_maps(contract, child))
}

fn rust_field_type(contract: &Contract, field: &Field, package: &str) -> String {
    if field.map {
        let entry = find_message(contract, field.target.as_deref().unwrap_or(""));
        let key = entry
            .fields
            .iter()
            .find(|f| f.number == 1)
            .map(|f| rust_scalar_type(contract, f, package))
            .unwrap_or_else(|| "()".into());
        let value = entry
            .fields
            .iter()
            .find(|f| f.number == 2)
            .map(|f| rust_scalar_type(contract, f, package))
            .unwrap_or_else(|| "()".into());
        return format!("BTreeMap<{key}, {value}>");
    }
    let ty = rust_scalar_type(contract, field, package);
    if field.repeated {
        format!("Vec<{ty}>")
    } else if field.optional {
        format!("Option<{ty}>")
    } else {
        ty
    }
}

fn rust_scalar_type(contract: &Contract, field: &Field, package: &str) -> String {
    match field.kind.as_str() {
        "bool" => "bool".into(),
        "double" => "f64".into(),
        "float" => "f32".into(),
        "fixed32" | "uint32" => "u32".into(),
        "sfixed32" | "sint32" | "int32" | "enum" => "i32".into(),
        "fixed64" | "uint64" => "u64".into(),
        "sfixed64" | "sint64" | "int64" => "i64".into(),
        "bytes" => "Vec<u8>".into(),
        "string" => "String".into(),
        "message" => relative_rust_type(
            package,
            find_message(contract, field.target.as_deref().unwrap_or(""))
                .rust
                .trim_start_matches("messages::"),
        ),
        _ => "()".into(),
    }
}

fn relative_rust_type(package: &str, target: &str) -> String {
    let package_parts = package.split('.').collect::<Vec<_>>();
    let target_parts = target.split("::").collect::<Vec<_>>();
    let common = package_parts
        .iter()
        .zip(target_parts.iter())
        .take_while(|(left, right)| left == right)
        .count();
    let up = package_parts.len().saturating_sub(common);
    let mut parts = Vec::with_capacity(up + target_parts.len().saturating_sub(common));
    parts.extend(std::iter::repeat_n("super", up));
    parts.extend(target_parts[common..].iter().copied());
    parts.join("::")
}

fn find_message<'a>(contract: &'a Contract, name: &str) -> &'a Message {
    contract
        .messages
        .iter()
        .find(|m| m.name == name)
        .unwrap_or(&contract.messages[0])
}

fn emit_rust_codec(out: &mut String, contract: &Contract, message: &Message, name: &str) {
    if message.map_entry {
        return;
    }
    writeln!(out, "impl IpcCodec for {name} {{").ok();
    out.push_str(
        "fn encode_ipc(&self) -> Vec<u8> { let mut writer = PayloadWriter::with_capacity(64);\n",
    );
    for field in &message.fields {
        emit_rust_encode_field(out, contract, message, field);
    }
    out.push_str("writer.finish() }\n");
    writeln!(out, "fn decode_ipc(bytes: &[u8]) -> Result<Self, IpcError> {{ let mut value = Self::default(); let mut reader = WireReader::new(bytes); while let Some(field) = reader.next_field()? {{ match field.number {{").ok();
    for field in &message.fields {
        emit_rust_decode_field(out, contract, message, field);
    }
    out.push_str("_ => {}, } } Ok(value) }\n}\n");
}

fn emit_rust_encode_field(out: &mut String, contract: &Contract, message: &Message, field: &Field) {
    let name = rust_ident(&snake(&field.name));
    if let Some(oneof) = &field.oneof {
        let enum_name = pascal(oneof);
        let module = rust_ident(&snake(
            message.name.rsplit('.').next().unwrap_or(&message.name),
        ));
        let variant = pascal(&field.name);
        writeln!(
            out,
            "if let Some({module}::{enum_name}::{variant}(v)) = &self.{} {{ {} }}",
            rust_ident(&snake(oneof)),
            rust_write(contract, field, "v")
        )
        .ok();
        return;
    }
    if field.map {
        let entry = find_message(contract, field.target.as_deref().unwrap_or(""));
        let key = entry.fields.iter().find(|f| f.number == 1).unwrap_or(field);
        let val = entry.fields.iter().find(|f| f.number == 2).unwrap_or(field);
        writeln!(
            out,
            "for (k, v) in &self.{name} {{ let mut entry = PayloadWriter::with_capacity(32); {} {} writer.bytes({}, &entry.finish()); }}",
            rust_write_entry_component(contract, key, "k"),
            rust_write_entry_component(contract, val, "v"),
            field.number
        )
        .ok();
        return;
    }
    if field.repeated {
        if field.packed
            && field.kind != "string"
            && field.kind != "bytes"
            && field.kind != "message"
        {
            writeln!(out, "if !self.{name}.is_empty() {{ let mut packed = PayloadWriter::with_capacity(self.{name}.len()); for v in &self.{name} {{ {} }} writer.bytes({}, &packed.finish()); }}", rust_write_value_only(contract, field, "v", "packed"), field.number).ok();
        } else {
            writeln!(
                out,
                "for v in &self.{name} {{ {} }}",
                rust_write(contract, field, "v")
            )
            .ok();
        }
        return;
    }
    if field.optional {
        writeln!(
            out,
            "if let Some(v) = &self.{name} {{ {} }}",
            rust_write(contract, field, "v")
        )
        .ok();
        return;
    }
    let default = rust_default_check(field, &format!("self.{name}"));
    writeln!(
        out,
        "if {default} {{ {} }}",
        rust_write(contract, field, &format!("self.{name}"))
    )
    .ok();
}

fn rust_default_check(field: &Field, expr: &str) -> String {
    match field.kind.as_str() {
        "bool" => expr.to_owned(),
        "string" | "bytes" => format!("!{expr}.is_empty()"),
        "double" | "float" => format!("{expr} != 0.0"),
        _ => format!("{expr} != 0"),
    }
}

fn rust_write(contract: &Contract, field: &Field, expr: &str) -> String {
    rust_write_with_writer(contract, field, expr, "writer")
}

fn rust_write_value_only(contract: &Contract, field: &Field, expr: &str, writer: &str) -> String {
    match field.kind.as_str() {
        "bool" | "uint32" | "uint64" | "int32" | "int64" | "sint32" | "sint64" | "enum" => {
            format!("{writer}.varint({});", rust_varint_expr(field, expr))
        }
        "fixed32" | "sfixed32" | "float" => {
            format!("{writer}.raw_fixed32({} as u32);", rust_value(expr))
        }
        "fixed64" | "sfixed64" | "double" => {
            format!("{writer}.raw_fixed64({} as u64);", rust_value(expr))
        }
        _ => rust_write_with_writer(contract, field, expr, writer),
    }
}

fn rust_write_with_writer(_contract: &Contract, field: &Field, expr: &str, writer: &str) -> String {
    match field.kind.as_str() {
        "bool" => format!("{writer}.bool({}, {});", field.number, rust_value(expr)),
        "uint32" => format!("{writer}.uint32({}, {});", field.number, rust_value(expr)),
        "uint64" => format!("{writer}.uint64({}, {});", field.number, rust_value(expr)),
        "int32" | "enum" => format!("{writer}.int32({}, {});", field.number, rust_value(expr)),
        "int64" => format!("{writer}.int64({}, {});", field.number, rust_value(expr)),
        "sint32" => format!("{writer}.sint32({}, {});", field.number, rust_value(expr)),
        "sint64" => format!("{writer}.sint64({}, {});", field.number, rust_value(expr)),
        "fixed32" => format!("{writer}.fixed32({}, {});", field.number, rust_value(expr)),
        "fixed64" => format!("{writer}.fixed64({}, {});", field.number, rust_value(expr)),
        "sfixed32" => format!("{writer}.sfixed32({}, {});", field.number, rust_value(expr)),
        "sfixed64" => format!("{writer}.sfixed64({}, {});", field.number, rust_value(expr)),
        "float" => format!("{writer}.float({}, {});", field.number, rust_value(expr)),
        "double" => format!("{writer}.double({}, {});", field.number, rust_value(expr)),
        "bytes" => format!("{writer}.bytes({}, {});", field.number, rust_ref(expr)),
        "string" => format!("{writer}.string({}, {});", field.number, rust_ref(expr)),
        "message" => format!("{writer}.bytes({}, &({expr}).encode_ipc());", field.number),
        _ => String::new(),
    }
}

fn rust_write_entry_component(contract: &Contract, field: &Field, expr: &str) -> String {
    let write = rust_write_with_writer(contract, field, expr, "entry");
    if field.kind == "message" {
        write
    } else {
        format!("if {} {{ {write} }}", rust_present_check(field, expr))
    }
}

fn rust_present_check(field: &Field, expr: &str) -> String {
    match field.kind.as_str() {
        "bool" => rust_value(expr),
        "string" | "bytes" => format!("!{}.is_empty()", rust_ref(expr)),
        "double" | "float" => format!("{} != 0.0", rust_value(expr)),
        _ => format!("{} != 0", rust_value(expr)),
    }
}

fn rust_varint_expr(field: &Field, expr: &str) -> String {
    let value = rust_value(expr);
    match field.kind.as_str() {
        "sint32" => format!("(({value} << 1) ^ ({value} >> 31)) as u32 as u64"),
        "sint64" => format!("(({value} << 1) ^ ({value} >> 63)) as u64"),
        "int32" | "enum" => format!("({value} as i64 as u64)"),
        "int64" => format!("({value} as u64)"),
        "bool" => format!("u64::from({value})"),
        _ => format!("({value} as u64)"),
    }
}

fn rust_value(expr: &str) -> String {
    if expr.starts_with("self.") {
        expr.to_owned()
    } else {
        format!("*{expr}")
    }
}

fn rust_ref(expr: &str) -> String {
    if expr.starts_with("self.") {
        format!("&{expr}")
    } else {
        expr.to_owned()
    }
}

fn emit_rust_decode_field(out: &mut String, contract: &Contract, message: &Message, field: &Field) {
    let target = if let Some(oneof) = &field.oneof {
        let variant = pascal(&field.name);
        let module = rust_ident(&snake(
            message.name.rsplit('.').next().unwrap_or(&message.name),
        ));
        format!(
            "value.{} = Some({module}::{}::{variant}({}));",
            rust_ident(&snake(oneof)),
            pascal(oneof),
            rust_read(contract, field, "field")
        )
    } else {
        let name = rust_ident(&snake(&field.name));
        if field.map {
            let entry = find_message(contract, field.target.as_deref().unwrap_or(""));
            let key_field = entry.fields.iter().find(|f| f.number == 1).unwrap_or(field);
            let value_field = entry.fields.iter().find(|f| f.number == 2).unwrap_or(field);
            format!("let mut r = WireReader::new(field.bytes()?); let mut k = Default::default(); let mut v = Default::default(); while let Some(e) = r.next_field()? {{ match e.number {{ 1 => k = {}, 2 => v = {}, _ => {{}} }} }} value.{name}.insert(k, v);", rust_read(contract, key_field, "e"), rust_read(contract, value_field, "e"))
        } else if field.repeated {
            if field.packed
                && field.kind != "string"
                && field.kind != "bytes"
                && field.kind != "message"
            {
                format!("if field.wire == 2 {{ let mut packed = WireReader::new(field.bytes()?); while !packed.is_empty() {{ value.{name}.push({}); }} }} else {{ value.{name}.push({}); }}", rust_read_packed(field, "packed"), rust_read(contract, field, "field"))
            } else {
                format!(
                    "value.{name}.push({});",
                    rust_read(contract, field, "field")
                )
            }
        } else if field.optional {
            format!(
                "value.{name} = Some({});",
                rust_read(contract, field, "field")
            )
        } else {
            format!("value.{name} = {};", rust_read(contract, field, "field"))
        }
    };
    writeln!(out, "{} => {{ {target} }}", field.number).ok();
}

fn rust_read(_contract: &Contract, field: &Field, field_expr: &str) -> String {
    match field.kind.as_str() {
        "bool" => format!("{field_expr}.bool()?"),
        "uint32" => format!("{field_expr}.uint32()?"),
        "uint64" => format!("{field_expr}.uint64()?"),
        "int32" | "enum" => format!("{field_expr}.int32()?"),
        "int64" => format!("{field_expr}.int64()?"),
        "sint32" => format!("{field_expr}.sint32()?"),
        "sint64" => format!("{field_expr}.sint64()?"),
        "fixed32" => format!("{field_expr}.fixed32()?"),
        "fixed64" => format!("{field_expr}.fixed64()?"),
        "sfixed32" => format!("{field_expr}.sfixed32()?"),
        "sfixed64" => format!("{field_expr}.sfixed64()?"),
        "float" => format!("{field_expr}.float()?"),
        "double" => format!("{field_expr}.double()?"),
        "bytes" => format!("{field_expr}.bytes()?.to_vec()"),
        "string" => format!("{field_expr}.string()?"),
        "message" => format!("IpcCodec::decode_ipc({field_expr}.bytes()?)?"),
        _ => "Default::default()".into(),
    }
}

fn rust_read_packed(field: &Field, reader: &str) -> String {
    match field.kind.as_str() {
        "bool" => format!("{reader}.varint()? != 0"),
        "uint32" => format!("{reader}.varint()? as u32"),
        "uint64" => format!("{reader}.varint()?"),
        "int32" | "enum" => format!("{reader}.varint()? as i64 as i32"),
        "int64" => format!("{reader}.varint()? as i64"),
        "sint32" => format!(
            "{{ let n = {reader}.varint()? as u32; ((n >> 1) as i32) ^ (-((n & 1) as i32)) }}"
        ),
        "sint64" => {
            format!("{{ let n = {reader}.varint()?; ((n >> 1) as i64) ^ (-((n & 1) as i64)) }}")
        }
        "fixed32" => format!("{reader}.fixed32()?"),
        "sfixed32" => format!("{reader}.fixed32()? as i32"),
        "float" => format!("f32::from_bits({reader}.fixed32()?)"),
        "fixed64" => format!("{reader}.fixed64()?"),
        "sfixed64" => format!("{reader}.fixed64()? as i64"),
        "double" => format!("f64::from_bits({reader}.fixed64()?)"),
        _ => "Default::default()".into(),
    }
}

fn emit_rust_enum(out: &mut String, full: &str, values: &BTreeMap<String, i32>) {
    let name = pascal(full.rsplit('.').next().unwrap_or(full));
    writeln!(out, "#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]\n#[repr(i32)]\npub enum {name} {{").ok();
    let prefix = format!(
        "{}_",
        full.rsplit('.').next().unwrap_or(full).to_ascii_uppercase()
    );
    for (value, number) in values {
        let variant = value.strip_prefix(&prefix).unwrap_or(value);
        writeln!(out, "{} = {number},", pascal(&variant.to_ascii_lowercase())).ok();
    }
    out.push_str("}\n");
}

fn emit_ts_enum(out: &mut String, full: &str, values: &BTreeMap<String, i32>) {
    let name = pascal(full.rsplit('.').next().unwrap_or(full));
    writeln!(out, "export type {name} = number;").ok();
    writeln!(out, "export const {name} = {{").ok();
    for (value, number) in values {
        writeln!(out, "{}: {number},", ts_field(value)).ok();
    }
    writeln!(out, "}} as const;").ok();
}

fn emit_ts_interface(out: &mut String, contract: &Contract, message: &Message) {
    writeln!(out, "export interface {} {{", message.ts).ok();
    for field in message.fields.iter().filter(|f| f.oneof.is_none()) {
        writeln!(
            out,
            "{}{}: {};",
            ts_field(&field.name),
            if field.optional { "?" } else { "" },
            ts_field_type(contract, field)
        )
        .ok();
    }
    for oneof in oneofs(message) {
        write!(out, "{}?: ", camel(&oneof)).ok();
        let mut first = true;
        for field in message
            .fields
            .iter()
            .filter(|f| f.oneof.as_deref() == Some(&oneof))
        {
            if !first {
                out.push_str(" | ");
            }
            first = false;
            write!(
                out,
                "{{ $case: {:?}; value: {} }}",
                ts_field(&field.name),
                ts_scalar_type(contract, field)
            )
            .ok();
        }
        out.push_str(";\n");
    }
    out.push_str("}\n");
}

fn ts_field_type(contract: &Contract, field: &Field) -> String {
    if field.map {
        let entry = find_message(contract, field.target.as_deref().unwrap_or(""));
        let key = entry
            .fields
            .iter()
            .find(|f| f.number == 1)
            .map(|f| ts_scalar_type(contract, f))
            .unwrap_or_else(|| "never".into());
        let value = entry
            .fields
            .iter()
            .find(|f| f.number == 2)
            .map(|f| ts_scalar_type(contract, f))
            .unwrap_or_else(|| "never".into());
        return format!("Map<{key}, {value}>");
    }
    let ty = ts_scalar_type(contract, field);
    if field.repeated {
        format!("{ty}[]")
    } else {
        ty
    }
}

fn ts_scalar_type(contract: &Contract, field: &Field) -> String {
    match field.kind.as_str() {
        "bool" => "boolean".into(),
        "uint64" | "int64" | "sint64" | "fixed64" | "sfixed64" => "bigint".into(),
        "bytes" => "Uint8Array".into(),
        "string" => "string".into(),
        "message" => find_message(contract, field.target.as_deref().unwrap_or(""))
            .ts
            .clone(),
        _ => "number".into(),
    }
}

fn emit_ts_codec(out: &mut String, contract: &Contract, message: &Message) {
    writeln!(
        out,
        "function createBase{}(): {} {{ return {{",
        message.ts, message.ts
    )
    .ok();
    for field in message.fields.iter().filter(|f| f.oneof.is_none()) {
        if field.optional {
            writeln!(out, "{}: undefined,", ts_field(&field.name)).ok();
            continue;
        }
        writeln!(
            out,
            "{}: {},",
            ts_field(&field.name),
            ts_default(contract, field)
        )
        .ok();
    }
    for oneof in oneofs(message) {
        writeln!(out, "{}: undefined,", ts_field(&oneof)).ok();
    }
    out.push_str("}; }\n");
    writeln!(out, "export const {} = {{", message.ts).ok();
    writeln!(
        out,
        "encode(message: {0}): Uint8Array {{ const writer = new PayloadWriter();",
        message.ts
    )
    .ok();
    for field in &message.fields {
        emit_ts_encode_field(out, contract, field);
    }
    out.push_str("return writer.finish(); },\n");
    writeln!(out, "decode(bytes: Uint8Array): {} {{ const reader = new BoundaryReader(bytes); const message = createBase{}(); while (reader.pos < reader.len) {{ const [number, type] = reader.tag(); switch (number) {{", message.ts, message.ts).ok();
    for field in &message.fields {
        emit_ts_decode_field(out, contract, field);
    }
    out.push_str("default: skipField(reader, type); break; } } return message; }\n};\n");
}

fn ts_default(contract: &Contract, field: &Field) -> String {
    if field.map {
        return "new Map()".into();
    }
    if field.repeated {
        return "[]".into();
    }
    match field.kind.as_str() {
        "bool" => "false".into(),
        "uint64" | "int64" | "sint64" | "fixed64" | "sfixed64" => "0n".into(),
        "bytes" => "new Uint8Array(0)".into(),
        "string" => "\"\"".into(),
        "message" => format!(
            "createBase{}()",
            find_message(contract, field.target.as_deref().unwrap_or("")).ts
        ),
        _ => "0".into(),
    }
}

fn emit_ts_encode_field(out: &mut String, contract: &Contract, field: &Field) {
    let name = ts_field(&field.name);
    if let Some(oneof) = &field.oneof {
        let oneof_name = ts_field(oneof);
        writeln!(out, "if (message.{oneof_name}?.$case === {:?}) {{ const value = message.{oneof_name}.value; {} }}", name, ts_write(contract, field, "value")).ok();
        return;
    }
    if field.map {
        let entry = find_message(contract, field.target.as_deref().unwrap_or(""));
        let key = entry.fields.iter().find(|f| f.number == 1).unwrap_or(field);
        let val = entry.fields.iter().find(|f| f.number == 2).unwrap_or(field);
        writeln!(
            out,
            "for (const [key, value] of message.{name}) {{ const entry = new PayloadWriter(); {} {} writer.bytesField({}, entry.finish()); }}",
            ts_write_entry_component(contract, key, "key"),
            ts_write_entry_component(contract, val, "value"),
            field.number
        )
        .ok();
        return;
    }
    if field.repeated {
        writeln!(
            out,
            "for (const value of message.{name}) {{ {} }}",
            ts_write(contract, field, "value")
        )
        .ok();
        return;
    }
    if field.optional {
        writeln!(
            out,
            "if (message.{name} !== undefined) {{ {} }}",
            ts_write(contract, field, &format!("message.{name}"))
        )
        .ok();
        return;
    }
    writeln!(
        out,
        "if ({}) {{ {} }}",
        ts_default_check(field, &format!("message.{name}")),
        ts_write(contract, field, &format!("message.{name}"))
    )
    .ok();
}

fn ts_default_check(field: &Field, expr: &str) -> String {
    match field.kind.as_str() {
        "bool" => expr.into(),
        "string" => format!("{expr} !== \"\""),
        "bytes" => format!("{expr}.byteLength !== 0"),
        "uint64" | "int64" | "sint64" | "fixed64" | "sfixed64" => format!("{expr} !== 0n"),
        _ => format!("{expr} !== 0"),
    }
}

fn ts_write(contract: &Contract, field: &Field, expr: &str) -> String {
    ts_write_with_writer(contract, field, expr, "writer")
}

fn ts_write_with_writer(contract: &Contract, field: &Field, expr: &str, writer: &str) -> String {
    match field.kind.as_str() {
        "message" => format!(
            "{writer}.bytesField({}, {}.encode({expr}));",
            field.number,
            find_message(contract, field.target.as_deref().unwrap_or("")).ts
        ),
        "bytes" => format!("{writer}.bytesField({}, {expr});", field.number),
        "enum" => format!("{writer}.enum({}, {expr});", field.number),
        other => format!("{writer}.{other}({}, {expr});", field.number),
    }
}

fn ts_write_entry_component(contract: &Contract, field: &Field, expr: &str) -> String {
    let write = ts_write_with_writer(contract, field, expr, "entry");
    if field.kind == "message" {
        write
    } else {
        format!("if ({}) {{ {write} }}", ts_default_check(field, expr))
    }
}

fn emit_ts_decode_field(out: &mut String, contract: &Contract, field: &Field) {
    let name = ts_field(&field.name);
    let statement = if let Some(oneof) = &field.oneof {
        format!(
            "message.{} = {{ $case: {:?}, value: {} }};",
            ts_field(oneof),
            name,
            ts_read(contract, field, "reader")
        )
    } else if field.map {
        let entry = find_message(contract, field.target.as_deref().unwrap_or(""));
        let key = entry.fields.iter().find(|f| f.number == 1).unwrap_or(field);
        let val = entry.fields.iter().find(|f| f.number == 2).unwrap_or(field);
        format!("const entry = new BoundaryReader(readDelimited(reader)); let key = {}; let value = {}; while (entry.pos < entry.len) {{ const [entryNumber, entryType] = entry.tag(); switch (entryNumber) {{ case 1: key = {}; break; case 2: value = {}; break; default: skipField(entry, entryType); break; }} }} message.{name}.set(key, value);", ts_default(contract, key), ts_default(contract, val), ts_read(contract, key, "entry"), ts_read(contract, val, "entry"))
    } else if field.repeated {
        if field.packed
            && field.kind != "string"
            && field.kind != "bytes"
            && field.kind != "message"
        {
            format!("if (type === 2) {{ const packed = new BoundaryReader(readDelimited(reader)); while (packed.pos < packed.len) message.{name}.push({}); }} else {{ message.{name}.push({}); }}", ts_read(contract, field, "packed"), ts_read(contract, field, "reader"))
        } else {
            format!(
                "message.{name}.push({});",
                ts_read(contract, field, "reader")
            )
        }
    } else {
        format!("message.{name} = {};", ts_read(contract, field, "reader"))
    };
    writeln!(out, "case {}: {{ {statement} break; }}", field.number).ok();
}

fn ts_read(contract: &Contract, field: &Field, reader: &str) -> String {
    match field.kind.as_str() {
        "message" => format!(
            "{}.decode(readDelimited({reader}))",
            find_message(contract, field.target.as_deref().unwrap_or("")).ts
        ),
        "bytes" => format!("new Uint8Array(readDelimited({reader}))"),
        "string" => {
            format!("new TextDecoder('utf-8', {{ fatal: true }}).decode(readDelimited({reader}))")
        }
        "bool" => format!("readUint32({reader}) === 1"),
        "uint32" => format!("readUint32({reader})"),
        "int32" | "enum" => format!("Number(BigInt.asIntN(32, readUint64({reader})))"),
        "sint32" => format!(
            "(() => {{ const raw = readUint32({reader}); return ((raw >>> 1) ^ -(raw & 1)); }})()"
        ),
        "fixed32" => format!("{reader}.fixed32()"),
        "sfixed32" => format!("{reader}.sfixed32()"),
        "uint64" => format!("readUint64({reader})"),
        "int64" => format!("BigInt.asIntN(64, readUint64({reader}))"),
        "sint64" => format!("(() => {{ const raw = readUint64({reader}); return BigInt.asIntN(64, (raw >> 1n) ^ -(raw & 1n)); }})()"),
        "fixed64" => format!("{reader}.fixed64()"),
        "sfixed64" => format!("{reader}.sfixed64()"),
        "float" => format!("{reader}.float()"),
        "double" => format!("{reader}.double()"),
        _ => "undefined".into(),
    }
}
