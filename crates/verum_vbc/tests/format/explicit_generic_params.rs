use super::*;

// Frozen v2.15 wire layout, independent of both the current serializer and
// parser. Offsets: id 0, name 4, parent 8, visibility 9, flags 10,
// properties 11, bytecode range 13/17, locals/registers/stack 21/23/25,
// type/ordinary parameter counts 27/28, Concrete(Unit) 29..34, contexts 34,
// intrinsic/is_const/register hints/return spelling/parameter spellings/origin
// at 35..41. There are no generator fields or variable-length entries.
const V215_FUNCTION: [u8; 41] = [
    0x44, 0x33, 0x22, 0x11, // id 0x11223344
    0x88, 0x77, 0x66, 0x55, // name 0x55667788
    0, 1, 0, // no parent, public, no flags
    0, 0, // properties
    0, 0, 0, 0, 0, 0, 0, 0, // bytecode offset and length
    0, 0, 0, 0, 0, 0, // local, register and stack counts
    0, 0, // type and ordinary parameter counts
    1, 0, 0, 0, 0, // Concrete(Unit)
    0, // contexts
    0, 0, 0, 0, 0, 0, // v2.15 trailing fields described above
];
const FIRST_ID: u32 = 0x11223344;
const NEXT_ID: u32 = 0x12345678;

fn function_bytes(minor: u16) -> Vec<u8> {
    let mut bytes = V215_FUNCTION.to_vec();
    if minor >= 16 {
        bytes.push(0); // v2.16 explicit generic declaration count
    }
    if minor >= 20 {
        bytes.push(0); // v2.20 value-use receipt count
    }
    bytes
}

fn append_next_descriptor(bytes: &mut Vec<u8>, minor: u16) {
    let mut next = function_bytes(minor);
    next[..4].copy_from_slice(&NEXT_ID.to_le_bytes());
    bytes.extend_from_slice(&next);
}

#[test]
fn v215_function_table_does_not_consume_the_next_descriptor() {
    let mut old = function_bytes(15);
    assert_eq!(old.len(), 41);
    append_next_descriptor(&mut old, 15);
    let mut reader = Deserializer::new(&old);
    let mut header = VbcHeader::default();
    header.version_minor = 15;
    reader.header = Some(header);
    let first = reader.parse_function_descriptor().unwrap();
    assert_eq!(first.id.0, FIRST_ID);
    assert!(first.explicit_type_param_ids.is_empty());
    assert_eq!(reader.offset, 41);
    let second = reader.parse_function_descriptor().unwrap();
    assert_eq!(second.id.0, NEXT_ID);
    assert!(second.explicit_type_param_ids.is_empty());
    assert_eq!(reader.offset, 82);
}

#[test]
fn v216_truncated_or_oversized_generic_tail_is_rejected() {
    let mut bytes = function_bytes(16);
    assert_eq!(bytes.pop(), Some(0));
    for tail in [Vec::new(), vec![1], vec![1, 1], vec![1, 1, 0, 0, 1, 0]] {
        let mut malformed = bytes.clone();
        malformed.extend(tail);
        let mut reader = Deserializer::new(&malformed);
        reader.header = Some(VbcHeader { version_minor: 16, ..VbcHeader::default() });
        assert!(reader.parse_function_descriptor().is_err());
    }
    let mut malformed = bytes;
    crate::encoding::encode_varint((MAX_FN_TYPE_REF_PARAMS + 1) as u64, &mut malformed);
    let mut reader = Deserializer::new(&malformed);
    reader.header = Some(VbcHeader { version_minor: 16, ..VbcHeader::default() });
    assert!(matches!(
        reader.parse_function_descriptor(),
        Err(VbcError::TableTooLarge {
            field: "fn_explicit_type_param_count",
            ..
        })
    ));
}

#[test]
fn versioned_function_tails_stop_at_their_own_descriptor_boundary() {
    for (minor, one_len) in [(15, 41), (16, 42), (19, 42), (20, 43), (crate::format::VERSION_MINOR, 43)] {
        let mut bytes = function_bytes(minor);
        assert_eq!(bytes.len(), one_len);
        append_next_descriptor(&mut bytes, minor);
        let mut reader = Deserializer::new(&bytes);
        reader.header = Some(VbcHeader { version_minor: minor, ..VbcHeader::default() });
        for (expected, id) in [(one_len, FIRST_ID), (2 * one_len, NEXT_ID)] {
            let descriptor = reader.parse_function_descriptor().unwrap();
            assert_eq!(descriptor.id.0, id, "minor={minor}");
            assert_eq!(descriptor.name.0, 0x55667788);
            assert_eq!(descriptor.return_type, TypeRef::Concrete(TypeId::UNIT));
            assert!(descriptor.explicit_type_param_ids.is_empty());
            assert!(descriptor.value_uses.is_none());
            assert_eq!(reader.offset, expected, "minor={minor}");
        }
    }
}

#[test]
fn v220_receipt_tail_has_independent_truncation_and_count_bounds() {
    let mut prefix = function_bytes(20);
    assert_eq!(prefix.pop(), Some(0));
    let mut truncated = Deserializer::new(&prefix);
    truncated.header = Some(VbcHeader { version_minor: 20, ..VbcHeader::default() });
    assert!(truncated.parse_function_descriptor().is_err());

    crate::encoding::encode_varint((crate::value_use::MAX_VALUE_USES + 1) as u64, &mut prefix);
    let mut oversized = Deserializer::new(&prefix);
    oversized.header = Some(VbcHeader { version_minor: 20, ..VbcHeader::default() });
    assert!(matches!(oversized.parse_function_descriptor(), Err(VbcError::TableTooLarge {
        field: "fn_value_uses", ..
    })));
}
