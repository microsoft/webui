// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use objc2::runtime::AnyObject;
use objc2_foundation::{NSDictionary, NSNumber, NSString};

use crate::ipc::{DocumentActivation, Hello, IpcError, IpcErrorCode};
pub(super) use crate::native_ipc::{hello_reply_json as reply_json, NativeHello as HelloControl};

pub(super) const MAX_CONTROL_BYTES: usize = 4096;

pub(super) enum Control {
    Hello(HelloControl),
    Disconnect { generation: u64, token: String },
}

// Read at most a known number of UTF-16 units before allocating UTF-8. The
// Foundation infallible UTF-8 conversion cannot handle unpaired surrogates.
pub(super) fn bounded_string(text: &NSString, max: usize) -> Option<String> {
    let length = text.length();
    if length > max || max > MAX_CONTROL_BYTES {
        return None;
    }
    let mut units = Vec::with_capacity(length);
    for index in 0..length {
        units.push(text.characterAtIndex(index));
    }
    let result = String::from_utf16(&units).ok()?;
    (result.len() <= max).then_some(result)
}

fn field(fields: &NSDictionary, key: &str, max: usize) -> Option<String> {
    let object = fields.objectForKey(&NSString::from_str(key))?;
    bounded_string(object.downcast_ref::<NSString>()?, max)
}

fn number(fields: &NSDictionary, key: &str) -> Option<u32> {
    let object = fields.objectForKey(&NSString::from_str(key))?;
    let number = object.downcast_ref::<NSNumber>()?;
    bounded_string(&number.stringValue(), 10)?.parse().ok()
}

pub(super) fn decimal(value: &str) -> Option<u64> {
    if value.is_empty() || value.len() > 20 || value.starts_with('0') {
        return None;
    }
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then_some(())?;
    value.parse().ok()
}

pub(super) fn nonce(value: &str) -> Option<[u8; 16]> {
    if value.len() != 32 {
        return None;
    }
    let mut bytes = [0; 16];
    for (pair, byte) in value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .zip(bytes.iter_mut())
    {
        *byte = digit(pair[0])? * 16 + digit(pair[1])?;
    }
    Some(bytes)
}

fn digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

pub(super) fn decode(body: &AnyObject) -> Option<Control> {
    // WebKit creates NSDictionary containers for JS records. Verify every
    // value before use; never serialize/describe an untrusted native object.
    let fields = body.downcast_ref::<NSDictionary>()?;
    match field(fields, "kind", 10)?.as_str() {
        "disconnect" if fields.count() == 3 => {
            let token = field(fields, "token", 32)?;
            nonce(&token)?;
            Some(Control::Disconnect {
                generation: decimal(&field(fields, "generation", 20)?)?,
                token,
            })
        }
        "hello" if fields.count() == 9 => Some(Control::Hello(HelloControl {
            call_id: field(fields, "callId", 32).filter(|s| !s.is_empty())?,
            hello: Hello {
                wire_version: number(fields, "wireVersion")?,
                contract_name: field(fields, "contractName", 256)?,
                contract_major: number(fields, "contractMajor")?,
                schema_hash: field(fields, "schemaHash", 64)?,
            },
            proof: DocumentActivation {
                navigation: decimal(&field(fields, "navigation", 20)?)?,
                document_nonce: nonce(&field(fields, "documentNonce", 32)?)?,
                challenge: nonce(&field(fields, "challenge", 32)?)?,
            },
        })),
        _ => None,
    }
}

pub(super) fn error(code: IpcErrorCode) -> IpcError {
    IpcError::new(
        code,
        "native desktop IPC rejected the operation",
        "reload the trusted main document and use matching generated bindings",
    )
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use objc2::AnyThread;
    use std::ptr::NonNull;

    #[test]
    fn rejects_invalid_native_utf16_and_bounds_actual_utf8() {
        let units = [0xd800_u16];
        // SAFETY: Foundation copies the one valid code-unit allocation.
        let invalid = unsafe {
            NSString::initWithCharacters_length(
                NSString::alloc(),
                NonNull::from(&units[..]).cast(),
                1,
            )
        };
        assert!(bounded_string(&invalid, 32).is_none());
        assert!(bounded_string(&NSString::from_str("éé"), 3).is_none());
        assert_eq!(bounded_string(&NSString::from_str("😀"), 4).unwrap(), "😀");
        assert!(bounded_string(&NSString::from_str(&"x".repeat(4097)), 4096).is_none());
    }

    #[test]
    fn proof_scalars_are_canonical_and_bounded() {
        assert_eq!(decimal("18446744073709551615"), Some(u64::MAX));
        for invalid in ["", "0", "01", "-1", "+1", " 1", "18446744073709551616"] {
            assert_eq!(decimal(invalid), None);
        }
        assert_eq!(nonce(&"ab".repeat(16)), Some([0xab; 16]));
        assert_eq!(nonce(&"AB".repeat(16)), None);
        assert_eq!(nonce(&"a".repeat(31)), None);
    }

    #[test]
    fn rejects_non_objects_without_coercion() {
        assert!(decode(&NSString::from_str("{}")).is_none());
        let dictionary = NSDictionary::<NSString, AnyObject>::new();
        assert!(decode(&dictionary).is_none());
    }

    #[test]
    fn native_records_accept_only_bounded_known_control_fields() {
        use objc2_foundation::{NSData, NSJSONReadingOptions, NSJSONSerialization};
        let json = r#"{"kind":"hello","wireVersion":2,"contractName":"test","contractMajor":1,"schemaHash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","callId":"1","navigation":"7","documentNonce":"01010101010101010101010101010101","challenge":"02020202020202020202020202020202"}"#;
        let parse = |json: &str| {
            NSJSONSerialization::JSONObjectWithData_options_error(
                &NSData::from_vec(json.as_bytes().to_vec()),
                NSJSONReadingOptions::empty(),
            )
            .unwrap()
        };
        let Some(Control::Hello(hello)) = decode(&parse(json)) else {
            panic!("valid hello rejected");
        };
        let extra_fields =
            json.replacen("\"kind\":\"hello\"", "\"kind\":\"hello\",\"methods\":[]", 1);
        assert!(decode(&parse(&extra_fields)).is_none());
        assert_eq!(hello.proof.navigation, 7);
        assert_eq!(hello.proof.document_nonce, [1; 16]);
        assert!(decode(&parse(
            &json.replace("\"test\"", &format!("\"{}\"", "x".repeat(257)))
        ))
        .is_none());
        assert!(decode(&parse(&json.replace("\"test\"", "{}"))).is_none());
        assert!(decode(&parse(
            &json.replace("\"wireVersion\":2", "\"wireVersion\":2.5")
        ))
        .is_none());
        assert!(decode(&parse(
            &json.replace("\"navigation\":\"7\"", "\"navigation\":\"07\"")
        ))
        .is_none());
        assert!(matches!(
            decode(&parse(
                r#"{"kind":"disconnect","generation":"9","token":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#
            )),
            Some(Control::Disconnect { generation: 9, .. })
        ));
        assert!(decode(&parse(r#"{"kind":"disconnect","generation":"9"}"#)).is_none());
    }
}
