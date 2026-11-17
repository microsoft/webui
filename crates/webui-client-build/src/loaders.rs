// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::BTreeMap;
use std::path::Path;

use base64::Engine;
use rolldown::plugin::{HookLoadArgs, HookLoadOutput, HookLoadReturn, SharedLoadPluginContext};
use rolldown_common::{EmittedAsset, ModuleType};

use crate::{ClientBuildError, Loader, ScriptSyntax};

pub(super) enum Kind {
    Native(ModuleType),
    CssSheet,
    File,
    DataUrl,
}

pub(super) fn syntax(kind: &Kind) -> Option<ScriptSyntax> {
    match kind {
        Kind::Native(ModuleType::Js) => Some(ScriptSyntax::Js),
        Kind::Native(ModuleType::Jsx) => Some(ScriptSyntax::Jsx),
        Kind::Native(ModuleType::Ts) => Some(ScriptSyntax::Ts),
        Kind::Native(ModuleType::Tsx) => Some(ScriptSyntax::Tsx),
        _ => None,
    }
}

pub(super) fn kind(
    args: &HookLoadArgs<'_>,
    loaders: &BTreeMap<String, Loader>,
) -> Result<Kind, ClientBuildError> {
    if args
        .attributes
        .iter()
        .any(|(key, value)| key != "type" || !matches!(value, "css" | "text" | "json" | "bytes"))
    {
        return Err(ClientBuildError::Invalid(
            "unsupported browser import attributes; use only type: css, text or json".into(),
        ));
    }
    if args.attributes.get("type") == Some("css") {
        return Ok(Kind::CssSheet);
    }
    let attributes_type = args.attributes.module_type();
    if let Some(ty) = args.asserted_module_type.or(attributes_type.as_ref()) {
        return match ty {
            ModuleType::Asset => Ok(Kind::File),
            ModuleType::Binary => Err(ClientBuildError::Invalid(
                "byte-array imports are unsupported; use the file or dataurl loader".into(),
            )),
            _ => Ok(Kind::Native(ty.clone())),
        };
    }
    // Match compound extensions with the same longest-suffix precedence as Rolldown.
    let explicit = args
        .raw_id
        .match_indices('.')
        .find_map(|(offset, _)| loaders.get(&args.raw_id[offset..]));
    let extension = Path::new(args.raw_id)
        .extension()
        .and_then(|value| value.to_str());
    let loader = explicit.copied().unwrap_or(match extension {
        Some("ts" | "mts" | "cts") => Loader::Ts,
        Some("tsx") => Loader::Tsx,
        Some("jsx") => Loader::Jsx,
        Some("json") => Loader::Json,
        Some("css") => Loader::Css,
        _ => Loader::Js,
    });
    Ok(match loader {
        Loader::Js => Kind::Native(ModuleType::Js),
        Loader::Ts => Kind::Native(ModuleType::Ts),
        Loader::Tsx => Kind::Native(ModuleType::Tsx),
        Loader::Jsx => Kind::Native(ModuleType::Jsx),
        Loader::Json => Kind::Native(ModuleType::Json),
        Loader::Text => Kind::Native(ModuleType::Text),
        Loader::Css => return Err(ClientBuildError::Invalid(
            "Rolldown does not support CSS bundling; use a text loader or import with { type: \"css\" } for a CSSStyleSheet".into(),
        )),
        Loader::File => Kind::File,
        Loader::DataUrl => Kind::DataUrl,
    })
}

pub(super) fn load(
    context: &SharedLoadPluginContext,
    args: &HookLoadArgs<'_>,
    bytes: Vec<u8>,
    kind: Kind,
) -> HookLoadReturn {
    let (code, module_type) = match kind {
        Kind::Native(ty) => (String::from_utf8(bytes)?, ty),
        Kind::CssSheet => {
            let text = String::from_utf8(bytes)?;
            let literal = ascii_literal(&text)?;
            (format!("const sheet = new CSSStyleSheet(); sheet.replaceSync({literal}); export default sheet;"), ModuleType::Js)
        }
        Kind::DataUrl => {
            let mime = mime_guess::from_path(args.raw_id).first_or_octet_stream();
            let url = format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            );
            (
                format!("export default {};", ascii_literal(&url)?),
                ModuleType::Js,
            )
        }
        Kind::File => {
            let name = Path::new(args.raw_id)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("asset");
            let reference = context.emit_file(
                EmittedAsset {
                    name: Some(name.to_owned()),
                    original_file_name: Some(args.raw_id.to_owned()),
                    source: bytes.into(),
                    ..Default::default()
                },
                None,
                None,
            )?;
            context.associate_module_with_file_ref(args.id, &reference);
            (
                format!("export default import.meta.ROLLDOWN_FILE_URL_{reference};"),
                ModuleType::Js,
            )
        }
    };
    Ok(Some(HookLoadOutput {
        code: code.into(),
        module_type: Some(module_type),
        ..Default::default()
    }))
}

pub(super) fn ascii_literal(value: &str) -> Result<String, serde_json::Error> {
    use std::fmt::Write;
    let json = serde_json::to_string(value)?;
    if json.is_ascii() {
        return Ok(json);
    }
    let mut output = String::with_capacity(json.len());
    for ch in json.chars() {
        if ch.is_ascii() {
            output.push(ch);
        } else {
            let mut buffer = [0; 2];
            for unit in ch.encode_utf16(&mut buffer) {
                let _ = write!(output, "\\u{unit:04x}");
            }
        }
    }
    Ok(output)
}
