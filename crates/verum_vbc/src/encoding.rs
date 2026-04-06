//! Binary encoding primitives for VBC format.
//!
//! This module provides utilities for encoding and decoding:
//! - Variable-length integers (VarInt)
//! - Signed variable-length integers (SignedVarInt)
//! - Fixed-width integers
//! - Floating point numbers
//! - Registers and register ranges
//!
//! # VarInt Encoding
//!
//! Uses continuation bit encoding:
//! - Each byte has 7 data bits and 1 continuation bit (MSB)
//! - If continuation bit is 1, more bytes follow
//! - Values 0-127 encode in 1 byte
//!
//! ```text
//! 0xxxxxxx                    - 7 bits  (0-127)
//! 1xxxxxxx 0xxxxxxx           - 14 bits (128-16383)
//! 1xxxxxxx 1xxxxxxx 0xxxxxxx  - 21 bits
//! ... up to 9 bytes for 64-bit values
//! ```

use std::io::{Read, Write};

use crate::error::{VbcError, VbcResult};
use crate::instruction::{Reg, RegRange};

// ============================================================================
// VarInt Encoding
// ============================================================================

/// Encodes a u64 as a variable-length integer.
///
/// Returns the number of bytes written.
#[inline]
pub fn encode_varint(value: u64, output: &mut Vec<u8>) -> usize {
    let mut v = value;
    let start_len = output.len();

    while v >= 0x80 {
        output.push((v as u8) | 0x80);
        v >>= 7;
    }
    output.push(v as u8);

    output.len() - start_len
}

/// Encodes a u64 as a variable-length integer to a writer.
#[inline]
pub fn write_varint<W: Write>(value: u64, writer: &mut W) -> std::io::Result<usize> {
    let mut v = value;
    let mut count = 0;

    while v >= 0x80 {
        writer.write_all(&[(v as u8) | 0x80])?;
        v >>= 7;
        count += 1;
    }
    writer.write_all(&[v as u8])?;
    count += 1;

    Ok(count)
}

/// Decodes a variable-length integer from a byte slice.
///
/// Returns the value and the number of bytes consumed.
#[inline]
pub fn decode_varint(data: &[u8], offset: &mut usize) -> VbcResult<u64> {
    let mut result: u64 = 0;
    let mut shift: u32 = 0;
    let start = *offset;

    loop {
        if *offset >= data.len() {
            return Err(VbcError::eof(*offset, 1));
        }

        let byte = data[*offset];
        *offset += 1;

        // Add 7 bits to result
        result |= ((byte & 0x7F) as u64) << shift;

        // Check continuation bit
        if byte & 0x80 == 0 {
            return Ok(result);
        }

        shift += 7;
        if shift >= 64 {
            return Err(VbcError::VarIntOverflow { offset: start });
        }
    }
}

/// Decodes a variable-length integer from a reader.
#[inline]
pub fn read_varint<R: Read>(reader: &mut R) -> std::io::Result<u64> {
    let mut result: u64 = 0;
    let mut shift: u32 = 0;
    let mut byte = [0u8; 1];

    loop {
        reader.read_exact(&mut byte)?;

        result |= ((byte[0] & 0x7F) as u64) << shift;

        if byte[0] & 0x80 == 0 {
            return Ok(result);
        }

        shift += 7;
        if shift >= 64 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "VarInt overflow",
            ));
        }
    }
}

// ============================================================================
// Signed VarInt Encoding (ZigZag)
// ============================================================================

/// Encodes a signed i64 using ZigZag encoding + VarInt.
///
/// ZigZag maps signed integers to unsigned:
/// - 0 -> 0, -1 -> 1, 1 -> 2, -2 -> 3, 2 -> 4, ...
#[inline]
pub fn encode_signed_varint(value: i64, output: &mut Vec<u8>) -> usize {
    let zigzag = ((value << 1) ^ (value >> 63)) as u64;
    encode_varint(zigzag, output)
}

/// Decodes a signed VarInt using ZigZag decoding.
#[inline]
pub fn decode_signed_varint(data: &[u8], offset: &mut usize) -> VbcResult<i64> {
    let zigzag = decode_varint(data, offset)?;
    Ok(((zigzag >> 1) as i64) ^ (-((zigzag & 1) as i64)))
}

// ============================================================================
// Fixed-Width Encoding
// ============================================================================

/// Encodes a u16 in little-endian.
#[inline]
pub fn encode_u16(value: u16, output: &mut Vec<u8>) {
    output.extend_from_slice(&value.to_le_bytes());
}

/// Decodes a u16 in little-endian.
#[inline]
pub fn decode_u16(data: &[u8], offset: &mut usize) -> VbcResult<u16> {
    if *offset + 2 > data.len() {
        return Err(VbcError::eof(*offset, 2));
    }
    let bytes: [u8; 2] = data[*offset..*offset + 2].try_into()
        .map_err(|_| VbcError::eof(*offset, 2))?;
    *offset += 2;
    Ok(u16::from_le_bytes(bytes))
}

/// Encodes a u32 in little-endian.
#[inline]
pub fn encode_u32(value: u32, output: &mut Vec<u8>) {
    output.extend_from_slice(&value.to_le_bytes());
}

/// Decodes a u32 in little-endian.
#[inline]
pub fn decode_u32(data: &[u8], offset: &mut usize) -> VbcResult<u32> {
    if *offset + 4 > data.len() {
        return Err(VbcError::eof(*offset, 4));
    }
    let bytes: [u8; 4] = data[*offset..*offset + 4].try_into()
        .map_err(|_| VbcError::eof(*offset, 4))?;
    *offset += 4;
    Ok(u32::from_le_bytes(bytes))
}

/// Encodes a u64 in little-endian.
#[inline]
pub fn encode_u64(value: u64, output: &mut Vec<u8>) {
    output.extend_from_slice(&value.to_le_bytes());
}

/// Decodes a u64 in little-endian.
#[inline]
pub fn decode_u64(data: &[u8], offset: &mut usize) -> VbcResult<u64> {
    if *offset + 8 > data.len() {
        return Err(VbcError::eof(*offset, 8));
    }
    let bytes: [u8; 8] = data[*offset..*offset + 8].try_into()
        .map_err(|_| VbcError::eof(*offset, 8))?;
    *offset += 8;
    Ok(u64::from_le_bytes(bytes))
}

/// Encodes an i64 in little-endian.
#[inline]
pub fn encode_i64(value: i64, output: &mut Vec<u8>) {
    output.extend_from_slice(&value.to_le_bytes());
}

/// Decodes an i64 in little-endian.
#[inline]
pub fn decode_i64(data: &[u8], offset: &mut usize) -> VbcResult<i64> {
    if *offset + 8 > data.len() {
        return Err(VbcError::eof(*offset, 8));
    }
    let bytes: [u8; 8] = data[*offset..*offset + 8].try_into()
        .map_err(|_| VbcError::eof(*offset, 8))?;
    *offset += 8;
    Ok(i64::from_le_bytes(bytes))
}

/// Encodes an f64 in little-endian.
#[inline]
pub fn encode_f64(value: f64, output: &mut Vec<u8>) {
    output.extend_from_slice(&value.to_le_bytes());
}

/// Decodes an f64 in little-endian.
#[inline]
pub fn decode_f64(data: &[u8], offset: &mut usize) -> VbcResult<f64> {
    if *offset + 8 > data.len() {
        return Err(VbcError::eof(*offset, 8));
    }
    let bytes: [u8; 8] = data[*offset..*offset + 8].try_into()
        .map_err(|_| VbcError::eof(*offset, 8))?;
    *offset += 8;
    Ok(f64::from_le_bytes(bytes))
}

/// Decodes an f32 in little-endian.
#[inline]
pub fn decode_f32(data: &[u8], offset: &mut usize) -> VbcResult<f32> {
    if *offset + 4 > data.len() {
        return Err(VbcError::eof(*offset, 4));
    }
    let bytes: [u8; 4] = data[*offset..*offset + 4].try_into()
        .map_err(|_| VbcError::eof(*offset, 4))?;
    *offset += 4;
    Ok(f32::from_le_bytes(bytes))
}

/// Decodes a single byte.
#[inline]
pub fn decode_u8(data: &[u8], offset: &mut usize) -> VbcResult<u8> {
    if *offset >= data.len() {
        return Err(VbcError::eof(*offset, 1));
    }
    let byte = data[*offset];
    *offset += 1;
    Ok(byte)
}

// ============================================================================
// Register Encoding
// ============================================================================

/// Encodes a register reference.
///
/// - r0-r127: Single byte (0x00-0x7F)
/// - r128-r16383: Two bytes (0x80 | high7, low8)
#[inline]
pub fn encode_reg(reg: Reg, output: &mut Vec<u8>) {
    if reg.0 < 128 {
        output.push(reg.0 as u8);
    } else {
        output.push(0x80 | ((reg.0 >> 8) as u8));
        output.push(reg.0 as u8);
    }
}

/// Decodes a register reference.
#[inline]
pub fn decode_reg(data: &[u8], offset: &mut usize) -> VbcResult<Reg> {
    let byte = decode_u8(data, offset)?;
    if byte & 0x80 == 0 {
        Ok(Reg(byte as u16))
    } else {
        let high = ((byte & 0x7F) as u16) << 8;
        let low = decode_u8(data, offset)? as u16;
        Ok(Reg(high | low))
    }
}

/// Encodes a register range.
#[inline]
pub fn encode_reg_range(range: RegRange, output: &mut Vec<u8>) {
    encode_reg(range.start, output);
    output.push(range.count);
}

/// Decodes a register range.
#[inline]
pub fn decode_reg_range(data: &[u8], offset: &mut usize) -> VbcResult<RegRange> {
    let start = decode_reg(data, offset)?;
    let count = decode_u8(data, offset)?;
    Ok(RegRange { start, count })
}

// ============================================================================
// String Encoding
// ============================================================================

/// Encodes a string as length-prefixed UTF-8.
#[inline]
pub fn encode_string(s: &str, output: &mut Vec<u8>) {
    encode_varint(s.len() as u64, output);
    output.extend_from_slice(s.as_bytes());
}

/// Decodes a length-prefixed string.
#[inline]
pub fn decode_string(data: &[u8], offset: &mut usize) -> VbcResult<String> {
    let len = decode_varint(data, offset)? as usize;
    if *offset + len > data.len() {
        return Err(VbcError::eof(*offset, len));
    }
    let bytes = &data[*offset..*offset + len];
    *offset += len;

    String::from_utf8(bytes.to_vec()).map_err(|e| VbcError::InvalidUtf8 {
        offset: (*offset - len) as u32,
        error: e,
    })
}

/// Decodes raw bytes with length prefix.
#[inline]
pub fn decode_bytes(data: &[u8], offset: &mut usize) -> VbcResult<Vec<u8>> {
    let len = decode_varint(data, offset)? as usize;
    if *offset + len > data.len() {
        return Err(VbcError::eof(*offset, len));
    }
    let bytes = data[*offset..*offset + len].to_vec();
    *offset += len;
    Ok(bytes)
}

// ============================================================================
// Size Calculations
// ============================================================================

/// Returns the encoded size of a VarInt.
#[inline]
pub fn varint_size(value: u64) -> usize {
    if value == 0 {
        return 1;
    }
    let bits = 64 - value.leading_zeros();
    bits.div_ceil(7) as usize
}

/// Returns the encoded size of a signed VarInt.
#[inline]
pub fn signed_varint_size(value: i64) -> usize {
    let zigzag = ((value << 1) ^ (value >> 63)) as u64;
    varint_size(zigzag)
}

/// Returns the encoded size of a register.
#[inline]
pub fn reg_size(reg: Reg) -> usize {
    if reg.0 < 128 {
        1
    } else {
        2
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    // ========================================================================
    // VarInt Edge Case Tests
    // ========================================================================

    #[test]
    fn test_varint_encode_decode() {
        let test_values: &[u64] = &[
            0,
            1,
            127,
            128,
            255,
            256,
            16383,
            16384,
            1_000_000,
            u64::MAX >> 1,
        ];

        for &value in test_values {
            let mut encoded = Vec::new();
            encode_varint(value, &mut encoded);

            let mut offset = 0;
            let decoded = decode_varint(&encoded, &mut offset).unwrap();
            assert_eq!(decoded, value, "Failed for value {}", value);
            assert_eq!(offset, encoded.len());
        }
    }

    #[test]
    fn test_varint_edge_cases() {
        // Test boundary values around encoding transitions
        let edge_cases: &[(u64, usize)] = &[
            (0, 1),                 // Minimum value
            (1, 1),                 // Smallest positive
            (0x7F, 1),              // Max 1-byte value (127)
            (0x80, 2),              // Min 2-byte value (128)
            (0xFF, 2),              // 255
            (0x3FFF, 2),            // Max 2-byte value (16383)
            (0x4000, 3),            // Min 3-byte value (16384)
            (0x1FFFFF, 3),          // Max 3-byte value
            (0x200000, 4),          // Min 4-byte value
            (0xFFFFFFF, 4),         // Max 4-byte value
            (0x10000000, 5),        // Min 5-byte value
            (u32::MAX as u64, 5),   // u32::MAX
            (0x7FFFFFFFF, 5),       // Max 5-byte value
            (0x800000000, 6),       // Min 6-byte value
            (0x3FFFFFFFFFF, 6),     // Max 6-byte value
            (0x40000000000, 7),     // Min 7-byte value
            (0x1FFFFFFFFFFFF, 7),   // Max 7-byte value
            (0x2000000000000, 8),   // Min 8-byte value
            (0xFFFFFFFFFFFFFF, 8),  // Max 8-byte value
            (0x100000000000000, 9), // Min 9-byte value
            (u64::MAX, 10),         // Maximum u64 value
        ];

        for &(value, expected_size) in edge_cases {
            let mut encoded = Vec::new();
            let size = encode_varint(value, &mut encoded);
            assert_eq!(
                size, expected_size,
                "Unexpected encoding size for value {:#x}: got {} bytes, expected {}",
                value, size, expected_size
            );
            assert_eq!(
                encoded.len(),
                expected_size,
                "Buffer length mismatch for {:#x}",
                value
            );

            // Verify round-trip
            let mut offset = 0;
            let decoded = decode_varint(&encoded, &mut offset).unwrap();
            assert_eq!(decoded, value, "Round-trip failed for value {:#x}", value);
            assert_eq!(offset, expected_size);
        }
    }

    #[test]
    fn test_varint_u32_max() {
        let value = u32::MAX as u64;
        let mut encoded = Vec::new();
        encode_varint(value, &mut encoded);

        let mut offset = 0;
        let decoded = decode_varint(&encoded, &mut offset).unwrap();
        assert_eq!(decoded, value);
    }

    #[test]
    fn test_varint_u64_max() {
        let value = u64::MAX;
        let mut encoded = Vec::new();
        let size = encode_varint(value, &mut encoded);
        assert_eq!(size, 10); // u64::MAX requires 10 bytes

        let mut offset = 0;
        let decoded = decode_varint(&encoded, &mut offset).unwrap();
        assert_eq!(decoded, value);
    }

    #[test]
    fn test_varint_powers_of_two() {
        for shift in 0..64u32 {
            let value = 1u64 << shift;
            let mut encoded = Vec::new();
            encode_varint(value, &mut encoded);

            let mut offset = 0;
            let decoded = decode_varint(&encoded, &mut offset).unwrap();
            assert_eq!(decoded, value, "Failed for 2^{}", shift);
        }
    }

    #[test]
    fn test_varint_powers_of_two_minus_one() {
        for shift in 1..64u32 {
            let value = (1u64 << shift) - 1;
            let mut encoded = Vec::new();
            encode_varint(value, &mut encoded);

            let mut offset = 0;
            let decoded = decode_varint(&encoded, &mut offset).unwrap();
            assert_eq!(decoded, value, "Failed for 2^{} - 1", shift);
        }
    }

    // ========================================================================
    // VarInt Writer/Reader Tests
    // ========================================================================

    #[test]
    fn test_write_varint() {
        let test_values: &[u64] = &[0, 1, 127, 128, 255, 16383, 16384, u32::MAX as u64, u64::MAX];

        for &value in test_values {
            let mut buffer = Vec::new();
            let size = write_varint(value, &mut buffer).unwrap();

            // Compare with encode_varint
            let mut expected = Vec::new();
            let expected_size = encode_varint(value, &mut expected);

            assert_eq!(size, expected_size, "Size mismatch for {}", value);
            assert_eq!(buffer, expected, "Buffer mismatch for {}", value);
        }
    }

    #[test]
    fn test_read_varint() {
        let test_values: &[u64] = &[0, 1, 127, 128, 255, 16383, 16384, u32::MAX as u64, u64::MAX];

        for &value in test_values {
            let mut encoded = Vec::new();
            encode_varint(value, &mut encoded);

            let mut cursor = Cursor::new(&encoded);
            let decoded = read_varint(&mut cursor).unwrap();
            assert_eq!(decoded, value, "Failed for value {}", value);
        }
    }

    #[test]
    fn test_read_varint_eof() {
        let incomplete = [0x80]; // Continuation bit set but no more bytes
        let mut cursor = Cursor::new(&incomplete[..]);
        let result = read_varint(&mut cursor);
        assert!(result.is_err());
    }

    #[test]
    fn test_read_varint_overflow() {
        // 10 bytes with all continuation bits set followed by more continuation
        // This would exceed 64 bits
        let overflow_data: [u8; 11] = [
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00,
        ];
        let mut cursor = Cursor::new(&overflow_data[..]);
        let result = read_varint(&mut cursor);
        assert!(result.is_err());
    }

    // ========================================================================
    // Signed VarInt Tests (ZigZag Encoding)
    // ========================================================================

    #[test]
    fn test_signed_varint_encode_decode() {
        let test_values: &[i64] = &[
            0,
            1,
            -1,
            127,
            -128,
            1000,
            -1000,
            i64::MAX >> 1,
            i64::MIN >> 1,
        ];

        for &value in test_values {
            let mut encoded = Vec::new();
            encode_signed_varint(value, &mut encoded);

            let mut offset = 0;
            let decoded = decode_signed_varint(&encoded, &mut offset).unwrap();
            assert_eq!(decoded, value, "Failed for value {}", value);
        }
    }

    #[test]
    fn test_signed_varint_positive_values() {
        let positive_values: &[i64] = &[
            0,
            1,
            2,
            63,
            64,
            127,
            128,
            255,
            256,
            16383,
            16384,
            i32::MAX as i64,
            i64::MAX,
        ];

        for &value in positive_values {
            let mut encoded = Vec::new();
            encode_signed_varint(value, &mut encoded);

            let mut offset = 0;
            let decoded = decode_signed_varint(&encoded, &mut offset).unwrap();
            assert_eq!(decoded, value, "Failed for positive value {}", value);
        }
    }

    #[test]
    fn test_signed_varint_negative_values() {
        let negative_values: &[i64] = &[
            -1,
            -2,
            -64,
            -65,
            -128,
            -129,
            -256,
            -16384,
            -16385,
            i32::MIN as i64,
            i64::MIN,
        ];

        for &value in negative_values {
            let mut encoded = Vec::new();
            encode_signed_varint(value, &mut encoded);

            let mut offset = 0;
            let decoded = decode_signed_varint(&encoded, &mut offset).unwrap();
            assert_eq!(decoded, value, "Failed for negative value {}", value);
        }
    }

    #[test]
    fn test_signed_varint_zigzag_mapping() {
        // Verify ZigZag mapping: 0 -> 0, -1 -> 1, 1 -> 2, -2 -> 3, 2 -> 4, ...
        let pairs: &[(i64, u64)] = &[
            (0, 0),
            (-1, 1),
            (1, 2),
            (-2, 3),
            (2, 4),
            (-3, 5),
            (3, 6),
            (i64::MAX, u64::MAX - 1),
            (i64::MIN, u64::MAX),
        ];

        for &(signed, expected_zigzag) in pairs {
            let zigzag = ((signed << 1) ^ (signed >> 63)) as u64;
            assert_eq!(
                zigzag, expected_zigzag,
                "ZigZag mapping wrong for {}",
                signed
            );
        }
    }

    #[test]
    fn test_signed_varint_extremes() {
        // i64::MAX and i64::MIN
        for &value in &[i64::MAX, i64::MIN] {
            let mut encoded = Vec::new();
            encode_signed_varint(value, &mut encoded);

            let mut offset = 0;
            let decoded = decode_signed_varint(&encoded, &mut offset).unwrap();
            assert_eq!(decoded, value);
        }
    }

    // ========================================================================
    // VarInt Size Calculation Tests
    // ========================================================================

    #[test]
    fn test_varint_size() {
        assert_eq!(varint_size(0), 1);
        assert_eq!(varint_size(127), 1);
        assert_eq!(varint_size(128), 2);
        assert_eq!(varint_size(16383), 2);
        assert_eq!(varint_size(16384), 3);
    }

    #[test]
    fn test_varint_size_comprehensive() {
        let test_cases: &[(u64, usize)] = &[
            (0, 1),
            (1, 1),
            (0x7F, 1),
            (0x80, 2),
            (0x3FFF, 2),
            (0x4000, 3),
            (0x1FFFFF, 3),
            (0x200000, 4),
            (u32::MAX as u64, 5),
            (u64::MAX, 10),
        ];

        for &(value, expected_size) in test_cases {
            let calculated = varint_size(value);
            assert_eq!(
                calculated, expected_size,
                "varint_size({:#x}) = {}, expected {}",
                value, calculated, expected_size
            );

            // Also verify against actual encoding
            let mut encoded = Vec::new();
            encode_varint(value, &mut encoded);
            assert_eq!(
                encoded.len(),
                expected_size,
                "Actual encoding size for {:#x} differs from varint_size",
                value
            );
        }
    }

    #[test]
    fn test_signed_varint_size() {
        // Test size calculation for signed values
        let test_cases: &[(i64, usize)] = &[
            (0, 1),
            (-1, 1),
            (1, 1),
            (63, 1),
            (-64, 1),
            (64, 2),
            (-65, 2),
            (8191, 2),
            (-8192, 2),
        ];

        for &(value, expected_size) in test_cases {
            let calculated = signed_varint_size(value);
            assert_eq!(
                calculated, expected_size,
                "signed_varint_size({}) = {}, expected {}",
                value, calculated, expected_size
            );

            // Verify against actual encoding
            let mut encoded = Vec::new();
            encode_signed_varint(value, &mut encoded);
            assert_eq!(encoded.len(), expected_size);
        }
    }

    // ========================================================================
    // Fixed-Width Encoding Tests
    // ========================================================================

    #[test]
    fn test_fixed_width() {
        let mut buf = Vec::new();

        encode_u16(0x1234, &mut buf);
        encode_u32(0x12345678, &mut buf);
        encode_u64(0x123456789ABCDEF0, &mut buf);
        encode_f64(3.14159, &mut buf);

        let mut offset = 0;
        assert_eq!(decode_u16(&buf, &mut offset).unwrap(), 0x1234);
        assert_eq!(decode_u32(&buf, &mut offset).unwrap(), 0x12345678);
        assert_eq!(decode_u64(&buf, &mut offset).unwrap(), 0x123456789ABCDEF0);
        assert!((decode_f64(&buf, &mut offset).unwrap() - 3.14159).abs() < 1e-10);
    }

    #[test]
    fn test_decode_u8() {
        let data = [0x00, 0x01, 0x7F, 0x80, 0xFF];
        for (i, &expected) in data.iter().enumerate() {
            let mut offset = i;
            let decoded = decode_u8(&data, &mut offset).unwrap();
            assert_eq!(decoded, expected);
            assert_eq!(offset, i + 1);
        }
    }

    #[test]
    fn test_decode_u8_edge_cases() {
        // Test all possible u8 values
        for value in 0u8..=255 {
            let data = [value];
            let mut offset = 0;
            let decoded = decode_u8(&data, &mut offset).unwrap();
            assert_eq!(decoded, value);
        }
    }

    #[test]
    fn test_u16_encode_decode() {
        let test_values: &[u16] = &[0, 1, 0x00FF, 0x0100, 0x7FFF, 0x8000, 0xFFFF];

        for &value in test_values {
            let mut buf = Vec::new();
            encode_u16(value, &mut buf);
            assert_eq!(buf.len(), 2);

            // Verify little-endian encoding
            assert_eq!(buf[0], (value & 0xFF) as u8);
            assert_eq!(buf[1], (value >> 8) as u8);

            let mut offset = 0;
            let decoded = decode_u16(&buf, &mut offset).unwrap();
            assert_eq!(decoded, value);
            assert_eq!(offset, 2);
        }
    }

    #[test]
    fn test_u32_encode_decode() {
        let test_values: &[u32] = &[
            0, 1, 0xFF, 0x100, 0xFFFF, 0x10000, 0x7FFFFFFF, 0x80000000, 0xFFFFFFFF,
        ];

        for &value in test_values {
            let mut buf = Vec::new();
            encode_u32(value, &mut buf);
            assert_eq!(buf.len(), 4);

            let mut offset = 0;
            let decoded = decode_u32(&buf, &mut offset).unwrap();
            assert_eq!(decoded, value);
            assert_eq!(offset, 4);
        }
    }

    #[test]
    fn test_u64_encode_decode() {
        let test_values: &[u64] = &[
            0,
            1,
            0xFF,
            0xFFFF,
            0xFFFFFFFF,
            0x7FFFFFFFFFFFFFFF,
            0x8000000000000000,
            0xFFFFFFFFFFFFFFFF,
        ];

        for &value in test_values {
            let mut buf = Vec::new();
            encode_u64(value, &mut buf);
            assert_eq!(buf.len(), 8);

            let mut offset = 0;
            let decoded = decode_u64(&buf, &mut offset).unwrap();
            assert_eq!(decoded, value);
            assert_eq!(offset, 8);
        }
    }

    #[test]
    fn test_i64_encode_decode() {
        let test_values: &[i64] = &[
            0,
            1,
            -1,
            127,
            -128,
            i32::MAX as i64,
            i32::MIN as i64,
            i64::MAX,
            i64::MIN,
        ];

        for &value in test_values {
            let mut buf = Vec::new();
            encode_i64(value, &mut buf);
            assert_eq!(buf.len(), 8);

            let mut offset = 0;
            let decoded = decode_i64(&buf, &mut offset).unwrap();
            assert_eq!(decoded, value);
            assert_eq!(offset, 8);
        }
    }

    #[test]
    fn test_f64_encode_decode() {
        let test_values: &[f64] = &[
            0.0,
            -0.0,
            1.0,
            -1.0,
            std::f64::consts::PI,
            std::f64::consts::E,
            f64::MIN,
            f64::MAX,
            f64::MIN_POSITIVE,
            f64::EPSILON,
        ];

        for &value in test_values {
            let mut buf = Vec::new();
            encode_f64(value, &mut buf);
            assert_eq!(buf.len(), 8);

            let mut offset = 0;
            let decoded = decode_f64(&buf, &mut offset).unwrap();
            assert_eq!(decoded, value);
        }
    }

    #[test]
    fn test_f64_special_values() {
        // Test special floating point values
        let special_values: &[f64] = &[f64::INFINITY, f64::NEG_INFINITY];

        for &value in special_values {
            let mut buf = Vec::new();
            encode_f64(value, &mut buf);

            let mut offset = 0;
            let decoded = decode_f64(&buf, &mut offset).unwrap();
            assert_eq!(decoded, value);
        }

        // NaN requires special handling since NaN != NaN
        let mut buf = Vec::new();
        encode_f64(f64::NAN, &mut buf);
        let mut offset = 0;
        let decoded = decode_f64(&buf, &mut offset).unwrap();
        assert!(decoded.is_nan());
    }

    // ========================================================================
    // Error Handling Tests
    // ========================================================================

    #[test]
    fn test_eof_errors() {
        let short_data = [0x80]; // Incomplete VarInt
        let mut offset = 0;
        assert!(decode_varint(&short_data, &mut offset).is_err());

        let empty_data: &[u8] = &[];
        let mut offset = 0;
        assert!(decode_u32(empty_data, &mut offset).is_err());
    }

    #[test]
    fn test_decode_from_empty_buffer() {
        let empty: &[u8] = &[];

        // VarInt from empty
        let mut offset = 0;
        let result = decode_varint(empty, &mut offset);
        assert!(result.is_err());

        // u8 from empty
        let mut offset = 0;
        let result = decode_u8(empty, &mut offset);
        assert!(result.is_err());

        // u16 from empty
        let mut offset = 0;
        let result = decode_u16(empty, &mut offset);
        assert!(result.is_err());

        // u32 from empty
        let mut offset = 0;
        let result = decode_u32(empty, &mut offset);
        assert!(result.is_err());

        // u64 from empty
        let mut offset = 0;
        let result = decode_u64(empty, &mut offset);
        assert!(result.is_err());

        // i64 from empty
        let mut offset = 0;
        let result = decode_i64(empty, &mut offset);
        assert!(result.is_err());

        // f64 from empty
        let mut offset = 0;
        let result = decode_f64(empty, &mut offset);
        assert!(result.is_err());

        // String from empty
        let mut offset = 0;
        let result = decode_string(empty, &mut offset);
        assert!(result.is_err());

        // Register from empty
        let mut offset = 0;
        let result = decode_reg(empty, &mut offset);
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_buffer_too_short() {
        // u16 needs 2 bytes
        let one_byte: &[u8] = &[0x12];
        let mut offset = 0;
        assert!(decode_u16(one_byte, &mut offset).is_err());

        // u32 needs 4 bytes
        let three_bytes: &[u8] = &[0x12, 0x34, 0x56];
        let mut offset = 0;
        assert!(decode_u32(three_bytes, &mut offset).is_err());

        // u64 needs 8 bytes
        let seven_bytes: &[u8] = &[0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE];
        let mut offset = 0;
        assert!(decode_u64(seven_bytes, &mut offset).is_err());

        // i64 needs 8 bytes
        let mut offset = 0;
        assert!(decode_i64(seven_bytes, &mut offset).is_err());

        // f64 needs 8 bytes
        let mut offset = 0;
        assert!(decode_f64(seven_bytes, &mut offset).is_err());
    }

    #[test]
    fn test_varint_overflow_error() {
        // Create a VarInt that would overflow 64 bits
        // After 9 bytes with continuation bits, we have 63 bits; 10th byte can only have 1 bit
        // 11 bytes with continuation bits would be invalid
        let overflow_data: [u8; 10] = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x80];
        let mut offset = 0;
        let result = decode_varint(&overflow_data, &mut offset);
        assert!(result.is_err());

        // Verify the error type
        if let Err(VbcError::VarIntOverflow { .. }) = result {
            // Expected
        } else {
            panic!("Expected VarIntOverflow error");
        }
    }

    #[test]
    fn test_incomplete_varint() {
        // Various incomplete VarInts (continuation bit set at end)
        let incomplete_cases: &[&[u8]] = &[
            &[0x80],                         // 1 byte, needs more
            &[0x80, 0x80],                   // 2 bytes, needs more
            &[0xFF, 0xFF, 0xFF],             // 3 bytes, needs more
            &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF], // 5 bytes, needs more
        ];

        for &data in incomplete_cases {
            let mut offset = 0;
            let result = decode_varint(data, &mut offset);
            assert!(result.is_err(), "Expected error for incomplete VarInt");
        }
    }

    // ========================================================================
    // Register Encoding Tests
    // ========================================================================

    #[test]
    fn test_reg_encode_decode() {
        for idx in [0, 1, 127, 128, 255, 1000, Reg::MAX] {
            let reg = Reg(idx);
            let mut buf = Vec::new();
            encode_reg(reg, &mut buf);

            let mut offset = 0;
            let decoded = decode_reg(&buf, &mut offset).unwrap();
            assert_eq!(decoded, reg);
        }
    }

    #[test]
    fn test_reg_single_byte_encoding() {
        // Registers 0-127 should encode in 1 byte
        for idx in 0..128u16 {
            let reg = Reg(idx);
            let mut buf = Vec::new();
            encode_reg(reg, &mut buf);

            assert_eq!(buf.len(), 1, "Register {} should encode in 1 byte", idx);
            assert_eq!(buf[0], idx as u8);
            assert!(buf[0] & 0x80 == 0, "High bit should not be set");

            let mut offset = 0;
            let decoded = decode_reg(&buf, &mut offset).unwrap();
            assert_eq!(decoded, reg);
            assert_eq!(offset, 1);
        }
    }

    #[test]
    fn test_reg_two_byte_encoding() {
        // Registers 128-16383 should encode in 2 bytes
        let two_byte_regs: &[u16] = &[128, 129, 255, 256, 1000, 8191, 8192, 16382, 16383];

        for &idx in two_byte_regs {
            let reg = Reg(idx);
            let mut buf = Vec::new();
            encode_reg(reg, &mut buf);

            assert_eq!(buf.len(), 2, "Register {} should encode in 2 bytes", idx);
            assert!(
                buf[0] & 0x80 != 0,
                "High bit should be set for 2-byte encoding"
            );

            let mut offset = 0;
            let decoded = decode_reg(&buf, &mut offset).unwrap();
            assert_eq!(decoded, reg);
            assert_eq!(offset, 2);
        }
    }

    #[test]
    fn test_reg_all_single_byte_values() {
        for idx in 0..128u16 {
            let reg = Reg(idx);
            let mut buf = Vec::new();
            encode_reg(reg, &mut buf);

            let mut offset = 0;
            let decoded = decode_reg(&buf, &mut offset).unwrap();
            assert_eq!(decoded.0, idx);
        }
    }

    #[test]
    fn test_reg_max_value() {
        let reg = Reg(Reg::MAX);
        let mut buf = Vec::new();
        encode_reg(reg, &mut buf);

        let mut offset = 0;
        let decoded = decode_reg(&buf, &mut offset).unwrap();
        assert_eq!(decoded, reg);
        assert_eq!(decoded.0, Reg::MAX);
    }

    #[test]
    fn test_reg_size() {
        assert_eq!(reg_size(Reg(0)), 1);
        assert_eq!(reg_size(Reg(127)), 1);
        assert_eq!(reg_size(Reg(128)), 2);
        assert_eq!(reg_size(Reg(Reg::MAX)), 2);
    }

    // ========================================================================
    // Register Range Tests
    // ========================================================================

    #[test]
    fn test_reg_range() {
        let range = RegRange::new(Reg(5), 10);
        let mut buf = Vec::new();
        encode_reg_range(range, &mut buf);

        let mut offset = 0;
        let decoded = decode_reg_range(&buf, &mut offset).unwrap();
        assert_eq!(decoded.start, Reg(5));
        assert_eq!(decoded.count, 10);
    }

    #[test]
    fn test_reg_range_single_byte_start() {
        let range = RegRange::new(Reg(50), 5);
        let mut buf = Vec::new();
        encode_reg_range(range, &mut buf);

        // 1 byte for reg + 1 byte for count = 2 bytes
        assert_eq!(buf.len(), 2);

        let mut offset = 0;
        let decoded = decode_reg_range(&buf, &mut offset).unwrap();
        assert_eq!(decoded.start, Reg(50));
        assert_eq!(decoded.count, 5);
    }

    #[test]
    fn test_reg_range_two_byte_start() {
        let range = RegRange::new(Reg(200), 8);
        let mut buf = Vec::new();
        encode_reg_range(range, &mut buf);

        // 2 bytes for reg + 1 byte for count = 3 bytes
        assert_eq!(buf.len(), 3);

        let mut offset = 0;
        let decoded = decode_reg_range(&buf, &mut offset).unwrap();
        assert_eq!(decoded.start, Reg(200));
        assert_eq!(decoded.count, 8);
    }

    #[test]
    fn test_reg_range_edge_cases() {
        let test_cases: &[(Reg, u8)] = &[
            (Reg(0), 0),
            (Reg(0), 1),
            (Reg(0), 255),
            (Reg(127), 128),
            (Reg(128), 64),
            (Reg(Reg::MAX), 255),
        ];

        for &(start, count) in test_cases {
            let range = RegRange::new(start, count);
            let mut buf = Vec::new();
            encode_reg_range(range, &mut buf);

            let mut offset = 0;
            let decoded = decode_reg_range(&buf, &mut offset).unwrap();
            assert_eq!(decoded.start, start);
            assert_eq!(decoded.count, count);
        }
    }

    // ========================================================================
    // String Encoding Tests
    // ========================================================================

    #[test]
    fn test_string_encode_decode() {
        let test_strings = ["", "hello", "hello world", "🦀 Rust"];

        for s in test_strings {
            let mut buf = Vec::new();
            encode_string(s, &mut buf);

            let mut offset = 0;
            let decoded = decode_string(&buf, &mut offset).unwrap();
            assert_eq!(decoded, s);
        }
    }

    #[test]
    fn test_string_empty() {
        let mut buf = Vec::new();
        encode_string("", &mut buf);

        // Empty string should encode as single byte (length 0)
        assert_eq!(buf.len(), 1);
        assert_eq!(buf[0], 0);

        let mut offset = 0;
        let decoded = decode_string(&buf, &mut offset).unwrap();
        assert_eq!(decoded, "");
    }

    #[test]
    fn test_string_unicode() {
        let unicode_strings = [
            "Hello, 世界!",
            "Привет мир",
            "🎉🎊🎁",
            "日本語テスト",
            "α β γ δ ε",
        ];

        for s in unicode_strings {
            let mut buf = Vec::new();
            encode_string(s, &mut buf);

            let mut offset = 0;
            let decoded = decode_string(&buf, &mut offset).unwrap();
            assert_eq!(decoded, s);
        }
    }

    #[test]
    fn test_string_long() {
        // Test string longer than 127 bytes (requires 2-byte length)
        let long_string: String = "a".repeat(200);
        let mut buf = Vec::new();
        encode_string(&long_string, &mut buf);

        let mut offset = 0;
        let decoded = decode_string(&buf, &mut offset).unwrap();
        assert_eq!(decoded, long_string);
    }

    // ========================================================================
    // Bytes Encoding Tests
    // ========================================================================

    #[test]
    fn test_bytes_decode() {
        let test_cases: &[&[u8]] = &[&[], &[0x00], &[0xFF], &[0x01, 0x02, 0x03, 0x04, 0x05]];

        for &data in test_cases {
            let mut buf = Vec::new();
            encode_varint(data.len() as u64, &mut buf);
            buf.extend_from_slice(data);

            let mut offset = 0;
            let decoded = decode_bytes(&buf, &mut offset).unwrap();
            assert_eq!(decoded, data);
        }
    }

    // ========================================================================
    // Round-Trip Tests
    // ========================================================================

    #[test]
    fn test_varint_roundtrip_all_sizes() {
        // Test one value for each encoding size (1-10 bytes)
        let values: &[u64] = &[
            0,                 // 1 byte
            200,               // 2 bytes
            20000,             // 3 bytes
            2000000,           // 4 bytes
            200000000,         // 5 bytes
            20000000000,       // 6 bytes
            2000000000000,     // 7 bytes
            200000000000000,   // 8 bytes
            20000000000000000, // 9 bytes
            u64::MAX,          // 10 bytes
        ];

        for &value in values {
            let mut encoded = Vec::new();
            encode_varint(value, &mut encoded);

            let mut offset = 0;
            let decoded = decode_varint(&encoded, &mut offset).unwrap();
            assert_eq!(decoded, value);
            assert_eq!(offset, encoded.len());
        }
    }

    #[test]
    fn test_mixed_data_roundtrip() {
        // Encode multiple values of different types
        let mut buf = Vec::new();

        encode_varint(42, &mut buf);
        encode_u16(0x1234, &mut buf);
        encode_signed_varint(-100, &mut buf);
        encode_u32(0xDEADBEEF, &mut buf);
        encode_reg(Reg(255), &mut buf);
        encode_u64(0x123456789ABCDEF0, &mut buf);
        encode_string("test", &mut buf);
        encode_i64(-999999, &mut buf);
        encode_f64(2.71828, &mut buf);

        // Decode in same order
        let mut offset = 0;
        assert_eq!(decode_varint(&buf, &mut offset).unwrap(), 42);
        assert_eq!(decode_u16(&buf, &mut offset).unwrap(), 0x1234);
        assert_eq!(decode_signed_varint(&buf, &mut offset).unwrap(), -100);
        assert_eq!(decode_u32(&buf, &mut offset).unwrap(), 0xDEADBEEF);
        assert_eq!(decode_reg(&buf, &mut offset).unwrap(), Reg(255));
        assert_eq!(decode_u64(&buf, &mut offset).unwrap(), 0x123456789ABCDEF0);
        assert_eq!(decode_string(&buf, &mut offset).unwrap(), "test");
        assert_eq!(decode_i64(&buf, &mut offset).unwrap(), -999999);
        assert!((decode_f64(&buf, &mut offset).unwrap() - 2.71828).abs() < 1e-10);

        // Should have consumed entire buffer
        assert_eq!(offset, buf.len());
    }

    #[test]
    fn test_sequential_decoding_with_offset() {
        let mut buf = Vec::new();

        // Encode 5 varints
        for i in 0..5u64 {
            encode_varint(i * 100, &mut buf);
        }

        // Decode them sequentially, verifying offset advances correctly
        let mut offset = 0;
        for i in 0..5u64 {
            let value = decode_varint(&buf, &mut offset).unwrap();
            assert_eq!(value, i * 100);
        }
        assert_eq!(offset, buf.len());
    }

    #[test]
    fn test_offset_at_end_of_buffer() {
        let buf = vec![0x42]; // Single byte: varint 66

        // Decode once - should succeed
        let mut offset = 0;
        let value = decode_varint(&buf, &mut offset).unwrap();
        assert_eq!(value, 0x42);
        assert_eq!(offset, 1);

        // Try to decode again - should fail (offset at end)
        let result = decode_varint(&buf, &mut offset);
        assert!(result.is_err());
    }
}
