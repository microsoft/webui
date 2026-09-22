// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Fixed-layout binary codec for the framework-owned IPC envelope.
//!
//! `IpcFrame`/`WireError` are the only messages exchanged between the Rust host
//! and its own JavaScript renderer inside the *same* packaged binary: both ends
//! are built and versioned together, so there is no cross-process schema
//! evolution to support. A fixed byte layout (see `DESIGN.md`) replaces the
//! previously protobuf-encoded envelope: smaller generated code, faster
//! encode/decode, and a single decode pass instead of a lenient-format pre-scan
//! plus a full parse. Application-defined message payloads remain arbitrary
//! opaque bytes here and are still validated generically by [`super::validation`].

use std::string::FromUtf8Error;

/// A malformed or truncated fixed-layout envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("malformed ipc envelope")]
pub struct DecodeError;

impl From<FromUtf8Error> for DecodeError {
    fn from(_: FromUtf8Error) -> Self {
        Self
    }
}

/// The framework-owned envelope wrapping every IPC frame.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IpcFrame {
    pub version: u32,
    pub generation: u64,
    pub id: u64,
    pub kind: i32,
    pub method_id: u32,
    pub timeout_ms: u32,
    pub body: ::core::option::Option<ipc_frame::Body>,
}

/// Nested message and enum types in `IpcFrame`.
pub mod ipc_frame {
    use super::WireError;

    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    pub enum Body {
        Payload(::std::vec::Vec<u8>),
        Error(WireError),
    }
}

/// A bounded, plain-text diagnostic sent in place of a successful reply.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WireError {
    pub code: ::std::string::String,
    pub message: ::std::string::String,
    pub help: ::std::string::String,
    pub application_code: ::std::string::String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(i32)]
pub enum Kind {
    Unspecified = 0,
    Request = 1,
    Result = 2,
    Error = 3,
    Notify = 4,
    Accept = 5,
    Cancel = 6,
}

impl Kind {
    /// String value of the enum field names, stable and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            Self::Unspecified => "KIND_UNSPECIFIED",
            Self::Request => "REQUEST",
            Self::Result => "RESULT",
            Self::Error => "ERROR",
            Self::Notify => "NOTIFY",
            Self::Accept => "ACCEPT",
            Self::Cancel => "CANCEL",
        }
    }
    /// Creates an enum from its field name.
    pub fn from_str_name(value: &str) -> ::core::option::Option<Self> {
        match value {
            "KIND_UNSPECIFIED" => Some(Self::Unspecified),
            "REQUEST" => Some(Self::Request),
            "RESULT" => Some(Self::Result),
            "ERROR" => Some(Self::Error),
            "NOTIFY" => Some(Self::Notify),
            "ACCEPT" => Some(Self::Accept),
            "CANCEL" => Some(Self::Cancel),
            _ => None,
        }
    }
}

/// Rejects an unknown wire value rather than defaulting to a valid-looking kind.
impl ::core::convert::TryFrom<i32> for Kind {
    type Error = ();
    fn try_from(
        value: i32,
    ) -> ::core::result::Result<Self, <Self as ::core::convert::TryFrom<i32>>::Error> {
        match value {
            0 => Ok(Self::Unspecified),
            1 => Ok(Self::Request),
            2 => Ok(Self::Result),
            3 => Ok(Self::Error),
            4 => Ok(Self::Notify),
            5 => Ok(Self::Accept),
            6 => Ok(Self::Cancel),
            _ => Err(()),
        }
    }
}

/// Fixed header size in bytes, before any body trailer.
const HEADER_LEN: usize = 30;
const BODY_TAG_NONE: u8 = 0;
const BODY_TAG_PAYLOAD: u8 = 1;
const BODY_TAG_ERROR: u8 = 2;

/// Consumes and returns the next `N` bytes, or fails if too few remain.
fn take<'a>(input: &mut &'a [u8], len: usize) -> Result<&'a [u8], DecodeError> {
    if input.len() < len {
        return Err(DecodeError);
    }
    let (head, rest) = input.split_at(len);
    *input = rest;
    Ok(head)
}

fn read_array<const N: usize>(input: &mut &[u8]) -> Result<[u8; N], DecodeError> {
    let slice = take(input, N)?;
    let mut array = [0u8; N];
    array.copy_from_slice(slice);
    Ok(array)
}

fn read_u8(input: &mut &[u8]) -> Result<u8, DecodeError> {
    Ok(take(input, 1)?[0])
}

fn read_u32(input: &mut &[u8]) -> Result<u32, DecodeError> {
    Ok(u32::from_le_bytes(read_array::<4>(input)?))
}

fn read_u64(input: &mut &[u8]) -> Result<u64, DecodeError> {
    Ok(u64::from_le_bytes(read_array::<8>(input)?))
}

/// Reads a `[u32 length][bytes]` block, bounds-checked against what remains.
fn read_delimited<'a>(input: &mut &'a [u8]) -> Result<&'a [u8], DecodeError> {
    let len = read_u32(input)?;
    let len = usize::try_from(len).map_err(|_| DecodeError)?;
    take(input, len)
}

fn read_string(input: &mut &[u8]) -> Result<String, DecodeError> {
    Ok(String::from_utf8(read_delimited(input)?.to_vec())?)
}

/// Narrows a `Kind`-range `i32` to its wire byte, or `0` (`KIND_UNSPECIFIED`) if
/// out of range. An unspecified kind is always rejected by the caller's
/// semantic validation, so this can never silently produce a valid-looking frame.
fn kind_to_byte(kind: i32) -> u8 {
    u8::try_from(kind).unwrap_or(0)
}

fn push_u32(buf: &mut Vec<u8>, value: u32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn push_delimited(buf: &mut Vec<u8>, bytes: &[u8]) {
    push_u32(buf, u32::try_from(bytes.len()).unwrap_or(u32::MAX));
    buf.extend_from_slice(bytes);
}

impl WireError {
    /// Length of the standalone (headerless) encoding.
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        16 + self.code.len() + self.message.len() + self.help.len() + self.application_code.len()
    }

    fn write(&self, buf: &mut Vec<u8>) {
        push_delimited(buf, self.code.as_bytes());
        push_delimited(buf, self.message.as_bytes());
        push_delimited(buf, self.help.as_bytes());
        push_delimited(buf, self.application_code.as_bytes());
    }

    fn read(input: &mut &[u8]) -> Result<Self, DecodeError> {
        Ok(Self {
            code: read_string(input)?,
            message: read_string(input)?,
            help: read_string(input)?,
            application_code: read_string(input)?,
        })
    }

    /// Encodes the standalone (headerless) four-string form.
    #[must_use]
    pub fn encode_to_vec(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.encoded_len());
        self.write(&mut buf);
        buf
    }

    /// Decodes the standalone (headerless) four-string form, rejecting trailing bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut input = bytes;
        let value = Self::read(&mut input)?;
        if !input.is_empty() {
            return Err(DecodeError);
        }
        Ok(value)
    }
}

impl IpcFrame {
    /// Encoded length, computed without allocating - callers use this to check
    /// size limits before deciding whether to encode at all.
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        HEADER_LEN
            + match &self.body {
                None => 0,
                Some(ipc_frame::Body::Payload(bytes)) => 4 + bytes.len(),
                Some(ipc_frame::Body::Error(error)) => error.encoded_len(),
            }
    }

    /// Encodes the full envelope: fixed header, then a body-tag-selected trailer.
    #[must_use]
    pub fn encode_to_vec(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.encoded_len());
        push_u32(&mut buf, self.version);
        buf.extend_from_slice(&self.generation.to_le_bytes());
        buf.extend_from_slice(&self.id.to_le_bytes());
        buf.push(kind_to_byte(self.kind));
        push_u32(&mut buf, self.method_id);
        push_u32(&mut buf, self.timeout_ms);
        match &self.body {
            None => buf.push(BODY_TAG_NONE),
            Some(ipc_frame::Body::Payload(bytes)) => {
                buf.push(BODY_TAG_PAYLOAD);
                push_delimited(&mut buf, bytes);
            }
            Some(ipc_frame::Body::Error(error)) => {
                buf.push(BODY_TAG_ERROR);
                error.write(&mut buf);
            }
        }
        buf
    }

    /// Decodes a full envelope, rejecting truncated, malformed, or trailing bytes.
    ///
    /// The fixed layout has no ambiguity to police (no duplicate or unknown
    /// fields, no wire-type confusion): a single pass both parses and validates
    /// structural well-formedness, unlike the former lenient protobuf decoder,
    /// which needed a separate pre-scan pass before its own decode.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        if bytes.len() < HEADER_LEN {
            return Err(DecodeError);
        }
        let mut input = bytes;
        let version = read_u32(&mut input)?;
        let generation = read_u64(&mut input)?;
        let id = read_u64(&mut input)?;
        let kind = i32::from(read_u8(&mut input)?);
        let method_id = read_u32(&mut input)?;
        let timeout_ms = read_u32(&mut input)?;
        let body_tag = read_u8(&mut input)?;
        let body = match body_tag {
            BODY_TAG_NONE => None,
            BODY_TAG_PAYLOAD => Some(ipc_frame::Body::Payload(
                read_delimited(&mut input)?.to_vec(),
            )),
            BODY_TAG_ERROR => Some(ipc_frame::Body::Error(WireError::read(&mut input)?)),
            _ => return Err(DecodeError),
        };
        if !input.is_empty() {
            return Err(DecodeError);
        }
        Ok(Self {
            version,
            generation,
            id,
            kind,
            method_id,
            timeout_ms,
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_payload() -> IpcFrame {
        IpcFrame {
            version: 3,
            generation: 7,
            id: 42,
            kind: Kind::Request as i32,
            method_id: 1101,
            timeout_ms: 5000,
            body: Some(ipc_frame::Body::Payload(vec![1, 2, 3, 4, 5])),
        }
    }

    fn sample_error() -> IpcFrame {
        IpcFrame {
            version: 3,
            generation: 7,
            id: 42,
            kind: Kind::Error as i32,
            method_id: 0,
            timeout_ms: 0,
            body: Some(ipc_frame::Body::Error(WireError {
                code: "not-ready".into(),
                message: "not ready".into(),
                help: "retry later".into(),
                application_code: String::new(),
            })),
        }
    }

    fn sample_no_body() -> IpcFrame {
        IpcFrame {
            version: 3,
            generation: 7,
            id: 42,
            kind: Kind::Accept as i32,
            method_id: 0,
            timeout_ms: 0,
            body: None,
        }
    }

    #[test]
    fn round_trips_payload_error_and_no_body_frames() {
        for frame in [sample_payload(), sample_error(), sample_no_body()] {
            let bytes = frame.encode_to_vec();
            assert_eq!(bytes.len(), frame.encoded_len());
            let decoded = IpcFrame::decode(&bytes).unwrap();
            assert!(decoded == frame);
        }
    }

    #[test]
    fn round_trips_empty_payload_and_long_error_strings() {
        let empty = IpcFrame {
            body: Some(ipc_frame::Body::Payload(Vec::new())),
            ..sample_payload()
        };
        let bytes = empty.encode_to_vec();
        assert!(IpcFrame::decode(&bytes).unwrap() == empty);

        let long = IpcFrame {
            body: Some(ipc_frame::Body::Error(WireError {
                code: "x".repeat(2000),
                message: "y".repeat(2000),
                help: "z".repeat(2000),
                application_code: "w".repeat(2000),
            })),
            ..sample_error()
        };
        let bytes = long.encode_to_vec();
        assert!(IpcFrame::decode(&bytes).unwrap() == long);
    }

    #[test]
    fn standalone_wire_error_round_trips() {
        let error = WireError {
            code: "invalid-frame".into(),
            message: "invalid frame".into(),
            help: "check the connection".into(),
            application_code: "app-1".into(),
        };
        let bytes = error.encode_to_vec();
        assert!(WireError::decode(&bytes).unwrap() == error);
    }

    #[test]
    fn rejects_truncated_and_trailing_bytes() {
        let bytes = sample_payload().encode_to_vec();
        assert!(IpcFrame::decode(&bytes[..bytes.len() - 1]).is_err());
        assert!(IpcFrame::decode(&[]).is_err());
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(IpcFrame::decode(&trailing).is_err());
    }

    #[test]
    fn rejects_invalid_body_tag_and_overlong_length_prefix() {
        let mut bytes = sample_no_body().encode_to_vec();
        *bytes.last_mut().unwrap() = 9; // invalid body tag
        assert!(IpcFrame::decode(&bytes).is_err());

        // A payload length prefix claiming more bytes than remain.
        let mut payload = sample_payload().encode_to_vec();
        let len_offset = HEADER_LEN;
        payload[len_offset..len_offset + 4].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
        assert!(IpcFrame::decode(&payload).is_err());
    }

    #[test]
    fn kind_try_from_rejects_unknown_values() {
        assert!(Kind::try_from(0).is_ok());
        assert!(Kind::try_from(6).is_ok());
        assert!(Kind::try_from(7).is_err());
        assert!(Kind::try_from(-1).is_err());
    }
}
