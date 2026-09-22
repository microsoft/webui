// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use webkit6::javascriptcore as jsc;

use crate::ipc::{DocumentActivation, Hello, IpcError, IpcErrorCode};
use crate::native_ipc::NativeHello;

pub(super) const MAX_CONTROL_BYTES: usize = 4096;

pub(super) enum Control {
    Hello(NativeHello),
    Disconnect { generation: u64, token: String },
}

// WebKit delivers serialized values in a native JSC context, not live objects
// in the application document. Bound and validate that native value before
// asking JSC to allocate any Rust-owned UTF-8/JSON representation. No arbitrary
// object coercion, application payload conversion, or recursive walk is used.
const BOUNDED_CONTROL: &str = r#"(v)=>{
 'use strict';
 const text=(s,n)=>{
  if(typeof s!=='string'||s.length>n)return false;
  for(let i=0;i<s.length;i++){
   const c=s.charCodeAt(i);
   if(c>=0xd800&&c<=0xdbff){if(++i>=s.length)return false;const d=s.charCodeAt(i);if(d<0xdc00||d>0xdfff)return false;}
   else if(c>=0xdc00&&c<=0xdfff)return false;
  }
  return true;
 };
 if(!v||typeof v!=='object'||Array.isArray(v))return null;
 const hello=v.kind==='hello',disconnect=v.kind==='disconnect';
 if(!hello&&!disconnect)return null;
 const keys=hello?['kind','wireVersion','contractName','contractMajor','schemaHash','callId','navigation','documentNonce','challenge']:['kind','generation','token'];
 let count=0;for(const k in v){if(++count>keys.length||!keys.includes(k))return null;}
 if(count!==keys.length)return null;
 if(disconnect)return text(v.generation,20)&&text(v.token,32)?JSON.stringify({kind:'disconnect',generation:v.generation,token:v.token}):null;
 if(!Number.isInteger(v.wireVersion)||v.wireVersion<0||v.wireVersion>4294967295||
    !Number.isInteger(v.contractMajor)||v.contractMajor<0||v.contractMajor>4294967295||
    !text(v.contractName,256)||!text(v.schemaHash,64)||!text(v.callId,32)||v.callId.length===0||
    !text(v.navigation,20)||!text(v.documentNonce,32)||!text(v.challenge,32))return null;
 return JSON.stringify({kind:'hello',wireVersion:v.wireVersion,contractName:v.contractName,
  contractMajor:v.contractMajor,schemaHash:v.schemaHash,callId:v.callId,
  navigation:v.navigation,documentNonce:v.documentNonce,challenge:v.challenge});
}"#;

const BOUNDED_NONCE: &str = "(v)=>{ 'use strict'; if(typeof v!=='string'||v.length!==32)return null; for(let i=0;i<32;i++){const c=v.charCodeAt(i);if(!((c>=48&&c<=57)||(c>=97&&c<=102)))return null;} return v; }";

#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum RawControl {
    #[serde(rename_all = "camelCase")]
    Hello {
        wire_version: u32,
        contract_name: String,
        contract_major: u32,
        schema_hash: String,
        call_id: String,
        navigation: String,
        document_nonce: String,
        challenge: String,
    },
    Disconnect {
        generation: String,
        token: String,
    },
}

fn bounded_eval(value: &jsc::Value, script: &str) -> Option<String> {
    let context = value.context()?;
    let function = context.evaluate(script)?;
    let result = function.function_callv(std::slice::from_ref(value))?;
    if !result.is_string() {
        return None;
    }
    // The trusted validator has bounded every source field before conversion.
    let text = result.to_str();
    (text.len() <= MAX_CONTROL_BYTES).then(|| text.to_string())
}

pub(super) fn decode(value: &jsc::Value) -> Option<Control> {
    decode_json(&bounded_eval(value, BOUNDED_CONTROL)?)
}

fn decode_json(json: &str) -> Option<Control> {
    if json.len() > MAX_CONTROL_BYTES {
        return None;
    }
    match serde_json::from_str::<RawControl>(json).ok()? {
        RawControl::Hello {
            wire_version,
            contract_name,
            contract_major,
            schema_hash,
            call_id,
            navigation,
            document_nonce,
            challenge,
        } => Some(Control::Hello(NativeHello {
            call_id,
            hello: Hello {
                wire_version,
                contract_name,
                contract_major,
                schema_hash,
            },
            proof: DocumentActivation {
                navigation: decimal(&navigation)?,
                document_nonce: hex(&document_nonce)?,
                challenge: hex(&challenge)?,
            },
        })),
        RawControl::Disconnect { generation, token } => {
            hex(&token)?;
            Some(Control::Disconnect {
                generation: decimal(&generation)?,
                token,
            })
        }
    }
}

pub(super) fn nonce(value: &jsc::Value) -> Option<[u8; 16]> {
    hex(&bounded_eval(value, BOUNDED_NONCE)?)
}

fn decimal(value: &str) -> Option<u64> {
    if value.is_empty()
        || value.len() > 20
        || value.starts_with('0')
        || !value.bytes().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    value.parse().ok()
}

fn hex(value: &str) -> Option<[u8; 16]> {
    if value.len() != 32 {
        return None;
    }
    let mut result = [0; 16];
    for (pair, byte) in value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .zip(result.iter_mut())
    {
        *byte = digit(pair[0])? * 16 + digit(pair[1])?;
    }
    Some(result)
}

fn digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

pub(super) fn error(code: IpcErrorCode) -> IpcError {
    IpcError::new(
        code,
        "native desktop IPC operation rejected",
        "reload the trusted application document and use matching generated bindings",
    )
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn disconnect_requires_canonical_generation_and_secret() {
        assert!(decode_json(r#"{"kind":"disconnect","generation":"1"}"#).is_none());
        assert!(decode_json(
            r#"{"kind":"disconnect","generation":"01","token":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#
        )
        .is_none());
        assert!(matches!(
            decode_json(
                r#"{"kind":"disconnect","generation":"1","token":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#
            ),
            Some(Control::Disconnect { generation: 1, .. })
        ));
    }
}
