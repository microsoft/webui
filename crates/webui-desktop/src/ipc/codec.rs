// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{error::fail, IpcError, IpcErrorCode};

/// WebUI-owned binary payload codec implemented by generated IPC types.
pub trait IpcCodec: Sized + Default + Send + 'static {
    /// Encode this value into the generated binary payload format.
    fn encode_ipc(&self) -> Vec<u8>;
    /// Decode one value from generated binary payload bytes.
    fn decode_ipc(bytes: &[u8]) -> Result<Self, IpcError>;
}

impl IpcCodec for () {
    fn encode_ipc(&self) -> Vec<u8> {
        Vec::new()
    }

    fn decode_ipc(bytes: &[u8]) -> Result<Self, IpcError> {
        if !bytes.is_empty() {
            return Err(fail(IpcErrorCode::InvalidPayload));
        }
        Ok(())
    }
}

/// Compact protobuf-compatible writer used by generated payload codecs.
#[derive(Default)]
pub struct PayloadWriter {
    bytes: Vec<u8>,
}

impl PayloadWriter {
    /// Create a writer with an estimated encoded capacity.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(capacity),
        }
    }

    /// Finish this writer and return the encoded bytes.
    #[must_use]
    pub fn finish(self) -> Vec<u8> {
        self.bytes
    }

    /// Write a boolean field.
    pub fn bool(&mut self, number: u32, value: bool) {
        self.varint_field(number, u64::from(value));
    }

    /// Write an unsigned 32-bit field.
    pub fn uint32(&mut self, number: u32, value: u32) {
        self.varint_field(number, u64::from(value));
    }

    /// Write an unsigned 64-bit field.
    pub fn uint64(&mut self, number: u32, value: u64) {
        self.varint_field(number, value);
    }

    /// Write a signed 32-bit field using int32 encoding.
    pub fn int32(&mut self, number: u32, value: i32) {
        self.varint_field(number, i64::from(value).cast_unsigned());
    }

    /// Write a signed 64-bit field using int64 encoding.
    pub fn int64(&mut self, number: u32, value: i64) {
        self.varint_field(number, value.cast_unsigned());
    }

    /// Write a signed 32-bit field using zigzag encoding.
    pub fn sint32(&mut self, number: u32, value: i32) {
        self.varint_field(
            number,
            u64::from(((value << 1) ^ (value >> 31)).cast_unsigned()),
        );
    }

    /// Write a signed 64-bit field using zigzag encoding.
    pub fn sint64(&mut self, number: u32, value: i64) {
        self.varint_field(number, ((value << 1) ^ (value >> 63)).cast_unsigned());
    }

    /// Write a fixed 32-bit field.
    pub fn fixed32(&mut self, number: u32, value: u32) {
        self.key(number, 5);
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Write a fixed 64-bit field.
    pub fn fixed64(&mut self, number: u32, value: u64) {
        self.key(number, 1);
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Write a signed fixed 32-bit field.
    pub fn sfixed32(&mut self, number: u32, value: i32) {
        self.fixed32(number, value.cast_unsigned());
    }

    /// Write a signed fixed 64-bit field.
    pub fn sfixed64(&mut self, number: u32, value: i64) {
        self.fixed64(number, value.cast_unsigned());
    }

    /// Write a 32-bit float field.
    pub fn float(&mut self, number: u32, value: f32) {
        self.fixed32(number, value.to_bits());
    }

    /// Write a 64-bit float field.
    pub fn double(&mut self, number: u32, value: f64) {
        self.fixed64(number, value.to_bits());
    }

    /// Write a byte field.
    pub fn bytes(&mut self, number: u32, value: &[u8]) {
        self.key(number, 2);
        self.varint(value.len() as u64);
        self.bytes.extend_from_slice(value);
    }

    /// Write a UTF-8 string field.
    pub fn string(&mut self, number: u32, value: &str) {
        self.bytes(number, value.as_bytes());
    }

    fn varint_field(&mut self, number: u32, value: u64) {
        self.key(number, 0);
        self.varint(value);
    }

    fn key(&mut self, number: u32, wire: u8) {
        self.varint((u64::from(number) << 3) | u64::from(wire));
    }

    /// Write a raw varint value.
    pub fn varint(&mut self, mut value: u64) {
        while value >= 0x80 {
            let byte = u8::try_from(value & 0x7f).unwrap_or(0);
            self.bytes.push(byte | 0x80);
            value >>= 7;
        }
        self.bytes.push(u8::try_from(value).unwrap_or(0));
    }

    /// Write raw little-endian fixed32 data, without a field key.
    pub fn raw_fixed32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Write raw little-endian fixed64 data, without a field key.
    pub fn raw_fixed64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }
}

/// Iterator-style reader used by generated payload codecs.
pub struct WireReader<'a> {
    input: &'a [u8],
}

impl<'a> WireReader<'a> {
    /// Create a reader over encoded payload bytes.
    #[must_use]
    pub fn new(input: &'a [u8]) -> Self {
        Self { input }
    }

    /// Whether all bytes have been consumed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.input.is_empty()
    }

    /// Read the next field, returning `None` at end of input.
    pub fn next_field(&mut self) -> Result<Option<WireField<'a>>, IpcError> {
        if self.input.is_empty() {
            return Ok(None);
        }
        let key = read_varint(&mut self.input)?;
        let number = u32::try_from(key >> 3).map_err(|_| fail(IpcErrorCode::InvalidPayload))?;
        if number == 0 || number > 0x1fff_ffff {
            return Err(fail(IpcErrorCode::InvalidPayload));
        }
        let wire = u8::try_from(key & 7).map_err(|_| fail(IpcErrorCode::InvalidPayload))?;
        let data = read_field(&mut self.input, wire)?;
        Ok(Some(WireField { number, wire, data }))
    }

    /// Read one raw varint from a packed scalar field.
    pub fn varint(&mut self) -> Result<u64, IpcError> {
        read_varint(&mut self.input)
    }

    /// Read one raw fixed32 from a packed scalar field.
    pub fn fixed32(&mut self) -> Result<u32, IpcError> {
        let data = read_exact(&mut self.input, 4)?;
        Ok(u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
    }

    /// Read one raw fixed64 from a packed scalar field.
    pub fn fixed64(&mut self) -> Result<u64, IpcError> {
        let data = read_exact(&mut self.input, 8)?;
        Ok(u64::from_le_bytes([
            data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
        ]))
    }
}

/// One decoded wire field.
pub struct WireField<'a> {
    /// Field number.
    pub number: u32,
    /// Protobuf wire type.
    pub wire: u8,
    data: &'a [u8],
}

impl<'a> WireField<'a> {
    /// Decode a boolean.
    pub fn bool(&self) -> Result<bool, IpcError> {
        match self.uint32()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(fail(IpcErrorCode::InvalidPayload)),
        }
    }

    /// Decode an unsigned 32-bit integer.
    pub fn uint32(&self) -> Result<u32, IpcError> {
        u32::try_from(self.uint64()?).map_err(|_| fail(IpcErrorCode::InvalidPayload))
    }

    /// Decode an unsigned 64-bit integer.
    pub fn uint64(&self) -> Result<u64, IpcError> {
        self.expect_wire(0)?;
        let mut input = self.data;
        let value = read_varint(&mut input)?;
        if !input.is_empty() {
            return Err(fail(IpcErrorCode::InvalidPayload));
        }
        Ok(value)
    }

    /// Decode a signed 32-bit integer.
    pub fn int32(&self) -> Result<i32, IpcError> {
        let value = self.uint64()?;
        if value <= i32::MAX as u64 {
            return i32::try_from(value).map_err(|_| fail(IpcErrorCode::InvalidPayload));
        }
        if value >= 0xffff_ffff_8000_0000 {
            return i32::try_from(value.cast_signed())
                .map_err(|_| fail(IpcErrorCode::InvalidPayload));
        }
        Err(fail(IpcErrorCode::InvalidPayload))
    }

    /// Decode a signed 64-bit integer.
    pub fn int64(&self) -> Result<i64, IpcError> {
        Ok(self.uint64()?.cast_signed())
    }

    /// Decode a zigzag signed 32-bit integer.
    pub fn sint32(&self) -> Result<i32, IpcError> {
        let value = self.uint32()?;
        Ok((value >> 1).cast_signed() ^ (-(value & 1).cast_signed()))
    }

    /// Decode a zigzag signed 64-bit integer.
    pub fn sint64(&self) -> Result<i64, IpcError> {
        let value = self.uint64()?;
        Ok((value >> 1).cast_signed() ^ (-(value & 1).cast_signed()))
    }

    /// Decode a fixed 32-bit integer.
    pub fn fixed32(&self) -> Result<u32, IpcError> {
        self.expect_wire(5)?;
        if self.data.len() != 4 {
            return Err(fail(IpcErrorCode::InvalidPayload));
        }
        Ok(u32::from_le_bytes([
            self.data[0],
            self.data[1],
            self.data[2],
            self.data[3],
        ]))
    }

    /// Decode a fixed 64-bit integer.
    pub fn fixed64(&self) -> Result<u64, IpcError> {
        self.expect_wire(1)?;
        if self.data.len() != 8 {
            return Err(fail(IpcErrorCode::InvalidPayload));
        }
        Ok(u64::from_le_bytes([
            self.data[0],
            self.data[1],
            self.data[2],
            self.data[3],
            self.data[4],
            self.data[5],
            self.data[6],
            self.data[7],
        ]))
    }

    /// Decode a signed fixed 32-bit integer.
    pub fn sfixed32(&self) -> Result<i32, IpcError> {
        Ok(self.fixed32()?.cast_signed())
    }

    /// Decode a signed fixed 64-bit integer.
    pub fn sfixed64(&self) -> Result<i64, IpcError> {
        Ok(self.fixed64()?.cast_signed())
    }

    /// Decode a 32-bit float.
    pub fn float(&self) -> Result<f32, IpcError> {
        Ok(f32::from_bits(self.fixed32()?))
    }

    /// Decode a 64-bit float.
    pub fn double(&self) -> Result<f64, IpcError> {
        Ok(f64::from_bits(self.fixed64()?))
    }

    /// Borrow a length-delimited field.
    pub fn bytes(&self) -> Result<&'a [u8], IpcError> {
        self.expect_wire(2)?;
        Ok(self.data)
    }

    /// Decode a UTF-8 string.
    pub fn string(&self) -> Result<String, IpcError> {
        std::str::from_utf8(self.bytes()?)
            .map(str::to_owned)
            .map_err(|_| fail(IpcErrorCode::InvalidPayload))
    }

    fn expect_wire(&self, expected: u8) -> Result<(), IpcError> {
        if self.wire == expected {
            Ok(())
        } else {
            Err(fail(IpcErrorCode::InvalidPayload))
        }
    }
}

fn read_varint(input: &mut &[u8]) -> Result<u64, IpcError> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let Some((&byte, rest)) = input.split_first() else {
            return Err(fail(IpcErrorCode::InvalidPayload));
        };
        *input = rest;
        if shift == 63 && byte > 1 {
            return Err(fail(IpcErrorCode::InvalidPayload));
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Ok(value);
        }
    }
    Err(fail(IpcErrorCode::InvalidPayload))
}

fn read_exact<'a>(input: &mut &'a [u8], length: usize) -> Result<&'a [u8], IpcError> {
    if length > input.len() {
        return Err(fail(IpcErrorCode::InvalidPayload));
    }
    let (data, rest) = input.split_at(length);
    *input = rest;
    Ok(data)
}

fn read_field<'a>(input: &mut &'a [u8], wire: u8) -> Result<&'a [u8], IpcError> {
    match wire {
        0 => {
            let before = *input;
            read_varint(input)?;
            Ok(&before[..before.len() - input.len()])
        }
        1 => read_exact(input, 8),
        2 => {
            let length = usize::try_from(read_varint(input)?)
                .map_err(|_| fail(IpcErrorCode::InvalidPayload))?;
            read_exact(input, length)
        }
        5 => read_exact(input, 4),
        _ => Err(fail(IpcErrorCode::InvalidPayload)),
    }
}

#[cfg(test)]
mod tests {
    use super::IpcCodec;

    #[test]
    fn empty_payload_rejects_unknown_fields() {
        assert!(().encode_ipc().is_empty());
        assert!(<()>::decode_ipc(&[]).is_ok());
        assert!(<()>::decode_ipc(&[0x08, 0x01]).is_err());
    }
}
