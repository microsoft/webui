// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{
    error::fail,
    wire::{ipc_frame::Body, IpcFrame, Kind},
    IpcError, IpcErrorCode, IpcLimits, IPC_VERSION,
};

/// Generated scalar or message field metadata.
#[derive(Clone, Copy)]
pub enum FieldKind {
    /// Boolean varint.
    Bool,
    /// Signed int32.
    Int32,
    /// Signed int64.
    Int64,
    /// Unsigned int32.
    Uint32,
    /// Unsigned int64.
    Uint64,
    /// Zigzag int32.
    Sint32,
    /// Zigzag int64.
    Sint64,
    /// Unsigned fixed32.
    Fixed32,
    /// Unsigned fixed64.
    Fixed64,
    /// Signed fixed32.
    Sfixed32,
    /// Signed fixed64.
    Sfixed64,
    /// IEEE 32-bit float, including nonfinite values.
    Float,
    /// IEEE 64-bit float, including nonfinite values.
    Double,
    /// Enum numeric value, including unknown values.
    Enum,
    /// Opaque bytes.
    Bytes,
    /// UTF-8 text.
    String,
    /// Index into the generated message table.
    Message(usize),
}
/// One generated message's fields.
pub struct MessageShape {
    /// Schema fields, including map-entry messages.
    pub fields: &'static [FieldShape],
}
/// Generated validation metadata.
pub struct FieldShape {
    /// Protobuf field tag.
    pub number: u32,
    /// Expected wire encoding.
    pub kind: FieldKind,
    /// Whether collection entries must be counted, including packed values.
    pub repeated: bool,
    /// Whether a repeated scalar field accepts packed wire encoding.
    pub packed: bool,
    /// Local oneof group index.
    pub oneof: Option<u32>,
    /// True for the key field in a map-entry message.
    pub map_key: bool,
}

/// Validate wire boundaries before prost decoding, without recursion or allocating
/// declared payload lengths. Unknown fields are bounded and skipped.
pub fn validate_message(
    bytes: &[u8],
    root: usize,
    messages: &'static [MessageShape],
    limits: &IpcLimits,
) -> Result<(), IpcError> {
    if bytes.len() > limits.max_frame_bytes {
        return Err(fail(IpcErrorCode::PayloadTooLarge));
    }
    let shape = messages
        .get(root)
        .ok_or_else(|| fail(IpcErrorCode::SchemaMismatch))?;
    let mut stack = vec![Cursor::new(bytes, shape)];
    let mut entries = 0usize;
    while let Some(cursor) = stack.last_mut() {
        if cursor.input.is_empty() {
            stack.pop();
            continue;
        }
        let (number, wire) = key(&mut cursor.input)?;
        let data = field(&mut cursor.input, wire)?;
        let Some(rule) = cursor.shape.fields.iter().find(|f| f.number == number) else {
            continue;
        };
        if let Some(group) = rule.oneof {
            if cursor
                .oneofs
                .insert(group, number)
                .is_some_and(|old| old != number)
            {
                return Err(fail(IpcErrorCode::InvalidPayload));
            }
        }
        let scalar_wire = expected(rule.kind);
        if rule.repeated && rule.packed && wire == 2 && scalar_wire != 2 {
            let mut packed = data;
            while !packed.is_empty() {
                let value = field(&mut packed, scalar_wire)?;
                validate_scalar(value, rule.kind)?;
                entries += 1;
                if entries > limits.max_collection_entries_per_message {
                    return Err(fail(IpcErrorCode::PayloadTooLarge));
                }
            }
            continue;
        }
        if scalar_wire != wire {
            return Err(fail(IpcErrorCode::InvalidPayload));
        }
        if rule.repeated {
            entries = entries
                .checked_add(1)
                .ok_or_else(|| fail(IpcErrorCode::PayloadTooLarge))?;
            if entries > limits.max_collection_entries_per_message {
                return Err(fail(IpcErrorCode::PayloadTooLarge));
            }
        }
        match rule.kind {
            FieldKind::Message(child) => {
                if stack.len() >= limits.max_schema_depth {
                    return Err(fail(IpcErrorCode::PayloadTooLarge));
                }
                let shape = messages
                    .get(child)
                    .ok_or_else(|| fail(IpcErrorCode::SchemaMismatch))?;
                stack.push(Cursor::new(data, shape));
            }
            FieldKind::String => {
                // Generated browser codecs use Map, not object-backed maps.
                // Every legal protobuf string key remains representable.
                std::str::from_utf8(data).map_err(|_| fail(IpcErrorCode::InvalidPayload))?;
            }
            _ => validate_scalar(data, rule.kind)?,
        }
    }
    Ok(())
}

struct Cursor<'a> {
    input: &'a [u8],
    shape: &'static MessageShape,
    oneofs: std::collections::HashMap<u32, u32>,
}
impl<'a> Cursor<'a> {
    fn new(input: &'a [u8], shape: &'static MessageShape) -> Self {
        Self {
            input,
            shape,
            oneofs: std::collections::HashMap::new(),
        }
    }
}
fn expected(kind: FieldKind) -> u8 {
    match kind {
        FieldKind::Fixed64 | FieldKind::Sfixed64 | FieldKind::Double => 1,
        FieldKind::Fixed32 | FieldKind::Sfixed32 | FieldKind::Float => 5,
        FieldKind::Bytes | FieldKind::String | FieldKind::Message(_) => 2,
        _ => 0,
    }
}
fn validate_scalar(mut bytes: &[u8], kind: FieldKind) -> Result<(), IpcError> {
    if expected(kind) != 0 {
        return Ok(());
    }
    let value = varint(&mut bytes)?;
    let valid = match kind {
        FieldKind::Bool => value <= 1,
        FieldKind::Uint32 | FieldKind::Sint32 => value <= u64::from(u32::MAX),
        FieldKind::Int32 | FieldKind::Enum => {
            value <= i32::MAX as u64 || value >= 0xffff_ffff_8000_0000
        }
        _ => true,
    };
    if !valid {
        return Err(fail(IpcErrorCode::InvalidPayload));
    }
    Ok(())
}

pub(super) fn varint(input: &mut &[u8]) -> Result<u64, IpcError> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let Some((&byte, rest)) = input.split_first() else {
            return Err(fail(IpcErrorCode::InvalidFrame));
        };
        *input = rest;
        if shift == 63 && byte > 1 {
            return Err(fail(IpcErrorCode::InvalidFrame));
        }
        value |= u64::from(byte & 127) << shift;
        if byte < 128 {
            return Ok(value);
        }
    }
    Err(fail(IpcErrorCode::InvalidFrame))
}
fn key(input: &mut &[u8]) -> Result<(u32, u8), IpcError> {
    let key = varint(input)?;
    let number = u32::try_from(key >> 3).map_err(|_| fail(IpcErrorCode::InvalidFrame))?;
    if number == 0 || number > 0x1fff_ffff {
        return Err(fail(IpcErrorCode::InvalidFrame));
    }
    let wire = u8::try_from(key & 7).map_err(|_| fail(IpcErrorCode::InvalidFrame))?;
    Ok((number, wire))
}
fn field<'a>(input: &mut &'a [u8], wire: u8) -> Result<&'a [u8], IpcError> {
    let length = match wire {
        0 => {
            let before = *input;
            varint(input)?;
            return Ok(&before[..before.len() - input.len()]);
        }
        1 => 8,
        2 => usize::try_from(varint(input)?).map_err(|_| fail(IpcErrorCode::InvalidFrame))?,
        5 => 4,
        _ => return Err(fail(IpcErrorCode::InvalidFrame)),
    };
    if length > input.len() {
        return Err(fail(IpcErrorCode::InvalidFrame));
    }
    let (data, rest) = input.split_at(length);
    *input = rest;
    Ok(data)
}

/// Decode a strict v2 envelope after checking complete size and wire boundaries.
/// Application bytes remain opaque here; generated validators run on workers.
/// Decode a strict v3 envelope. The fixed layout has no ambiguity to police
/// (no duplicate/unknown fields, no wire-type confusion), so a single decode
/// pass both parses and structurally validates the bytes; only the envelope's
/// own semantic invariants are checked afterward. Application bytes remain
/// opaque here - generated validators run on workers.
pub fn decode_frame(bytes: &[u8], limits: &IpcLimits) -> Result<IpcFrame, IpcError> {
    if bytes.len() > limits.max_frame_bytes {
        return Err(fail(IpcErrorCode::PayloadTooLarge));
    }
    let frame = IpcFrame::decode(bytes).map_err(|_| fail(IpcErrorCode::InvalidFrame))?;
    validate_frame(&frame, limits)?;
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::{validate_message, FieldKind, FieldShape, MessageShape};
    use crate::ipc::IpcLimits;

    const REPEATED_INT32: [FieldShape; 1] = [FieldShape {
        number: 1,
        kind: FieldKind::Int32,
        repeated: true,
        packed: false,
        oneof: None,
        map_key: false,
    }];
    const PACKED_INT32: [FieldShape; 1] = [FieldShape {
        packed: true,
        ..REPEATED_INT32[0]
    }];
    const NON_PACKED_MESSAGES: [MessageShape; 1] = [MessageShape {
        fields: &REPEATED_INT32,
    }];
    const PACKED_MESSAGES: [MessageShape; 1] = [MessageShape {
        fields: &PACKED_INT32,
    }];

    #[test]
    fn repeated_scalar_respects_packed_metadata() {
        let packed_bytes = [0x0a, 0x02, 0x01, 0x02];
        assert!(
            validate_message(&packed_bytes, 0, &PACKED_MESSAGES, &IpcLimits::default()).is_ok()
        );
        assert!(validate_message(
            &packed_bytes,
            0,
            &NON_PACKED_MESSAGES,
            &IpcLimits::default()
        )
        .is_err());
    }
}

fn validate_frame(frame: &IpcFrame, limits: &IpcLimits) -> Result<(), IpcError> {
    if frame.version != IPC_VERSION {
        return Err(fail(IpcErrorCode::UnsupportedVersion));
    }
    if frame.generation == 0 || frame.id == 0 {
        return Err(fail(IpcErrorCode::InvalidFrame));
    }
    let kind = Kind::try_from(frame.kind).map_err(|_| fail(IpcErrorCode::InvalidFrame))?;
    let valid = match kind {
        Kind::Request => {
            frame.method_id != 0
                && frame.timeout_ms != 0
                && u64::from(frame.timeout_ms) <= limits.max_timeout_ms as u64
                && matches!(frame.body, Some(Body::Payload(_)))
        }
        Kind::Notify => {
            frame.method_id != 0
                && frame.timeout_ms == 0
                && matches!(frame.body, Some(Body::Payload(_)))
        }
        Kind::Result => {
            frame.method_id == 0
                && frame.timeout_ms == 0
                && matches!(frame.body, Some(Body::Payload(_)))
        }
        Kind::Error => {
            frame.method_id == 0
                && frame.timeout_ms == 0
                && matches!(frame.body, Some(Body::Error(_)))
        }
        Kind::Accept | Kind::Cancel => {
            frame.method_id == 0 && frame.timeout_ms == 0 && frame.body.is_none()
        }
        Kind::Unspecified => false,
    };
    if !valid {
        return Err(fail(IpcErrorCode::InvalidFrame));
    }
    if let Some(Body::Error(error)) = &frame.body {
        let total = error.code.len()
            + error.message.len()
            + error.help.len()
            + error.application_code.len();
        if total > limits.max_error_text_bytes_total {
            return Err(fail(IpcErrorCode::PayloadTooLarge));
        }
    }
    Ok(())
}
