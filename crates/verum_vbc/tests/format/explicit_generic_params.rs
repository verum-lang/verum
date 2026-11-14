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
    if minor >= 23 {
        bytes.push(0); // v2.23 optional semantic formal roster
    }
    if minor >= 27 {
        bytes.push(0); // v2.27 optional semantic result
    }
    if minor >= 30 {
        bytes.push(0); // v2.30 optional source-level signature
    }
    if minor >= 43 {
        bytes.push(0); // v2.43 optional callable declaration identity
    }
    if minor >= 58 {
        bytes.push(0); // v2.58 optional declared receiver
        bytes.push(0); // v2.58 optional source-definition identity
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
    for minor in 15..=crate::format::VERSION_MINOR {
        let one_len = match minor {
            15 => 41,
            16..=19 => 42,
            20..=22 => 43,
            23..=26 => 44,
            27..=29 => 45,
            30..=42 => 46,
            43..=57 => 47,
            _ => 49,
        };
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
            assert!(descriptor.semantic_return.is_none());
            assert!(descriptor.source_signature.is_none());
            assert!(descriptor.declaration_identity.is_none());
            assert!(descriptor.source_receiver.is_none());
            assert!(descriptor.source_definition.is_none());
            assert_eq!(reader.offset, expected, "minor={minor}");
        }
    }
}

// Frozen v2.58 suffix: receiver presence + Concrete(Int), then source
// identity presence + u16 version + 32 digest bytes + scope presence + digest.
// Deliberately independent of the production writer and source hash encoder.
fn v258_definition_bytes() -> verum_common::List<u8> {
    let mut bytes = verum_common::List::from_iter(function_bytes(57));
    bytes[10] = 0x08; // has_source_body
    bytes.extend([1, 1, 2, 0, 0, 0]); // present receiver, Concrete(Int)
    bytes.extend([1, 1, 0]); // present source definition, version 1
    bytes.extend([0x5a; 32]);
    bytes.push(1); // present implementation scope
    bytes.extend([0xa5; 32]);
    bytes
}

#[test]
fn v258_source_contract_tail_preserves_fields_and_next_descriptor() {
    let mut bytes = v258_definition_bytes();
    assert_eq!(bytes.len(), 121);
    let mut next = function_bytes(58);
    next[..4].copy_from_slice(&NEXT_ID.to_le_bytes());
    bytes.extend(next);
    let mut reader = Deserializer::new(&bytes);
    reader.header = Some(VbcHeader {
        version_minor: 58,
        ..VbcHeader::default()
    });
    let first = reader.parse_function_descriptor().unwrap();
    assert_eq!(first.id.0, FIRST_ID);
    assert_eq!(first.source_receiver, Some(TypeRef::Concrete(TypeId::INT)));
    assert!(first.has_source_body);
    let identity = first.source_definition.unwrap();
    assert_eq!(identity.version, 1);
    assert_eq!(identity.digest, [0x5a; 32]);
    assert_eq!(identity.implementation_scope, Some([0xa5; 32]));
    assert_eq!(reader.offset, 121);
    let next = reader.parse_function_descriptor().unwrap();
    assert_eq!(next.id.0, NEXT_ID);
    assert!(next.source_receiver.is_none());
    assert!(next.source_definition.is_none());
    assert_eq!(reader.offset, 170);
}

#[test]
fn v258_source_contract_tail_refuses_truncation_and_invalid_authority() {
    let bytes = v258_definition_bytes();
    for length in 47..bytes.len() {
        let mut reader = Deserializer::new(&bytes.as_slice()[..length]);
        reader.header = Some(VbcHeader {
            version_minor: 58,
            ..VbcHeader::default()
        });
        assert!(
            reader.parse_function_descriptor().is_err(),
            "truncated at {length}"
        );
    }
    // The source body's absence, invalid presence tags, unknown source hash
    // version and malformed receiver type cannot be treated as absent metadata.
    for (offset, value) in [(10, 0), (47, 2), (48, 255), (53, 2), (54, 2), (88, 2)] {
        let mut malformed = bytes.clone();
        malformed[offset] = value;
        let mut reader = Deserializer::new(&malformed);
        reader.header = Some(VbcHeader {
            version_minor: 58,
            ..VbcHeader::default()
        });
        assert!(
            reader.parse_function_descriptor().is_err(),
            "invalid byte at {offset}"
        );
    }
}

#[test]
fn v230_source_signature_tail_rejects_missing_and_malformed_bytes() {
    let prefix = function_bytes(29);
    for tail in [&[][..], &[2][..], &[1][..], &[1, 4, 0][..]] {
        let bytes = [prefix.as_slice(), tail].concat();
        let mut reader = Deserializer::new(&bytes);
        reader.header = Some(VbcHeader { version_minor: 30, ..VbcHeader::default() });
        assert!(reader.parse_function_descriptor().is_err(), "tail={tail:?}");
    }
}

#[test]
fn semantic_result_tail_preserves_the_source_type_and_descriptor_boundary() {
    let mut bytes = function_bytes(27);
    assert_eq!(bytes.pop(), Some(0));
    bytes.extend([1, 1, 2, 0, 0, 0]); // present, Concrete(Int), unlike the Unit ABI
    let boundary = bytes.len();
    append_next_descriptor(&mut bytes, 27);
    let mut reader = Deserializer::new(&bytes);
    reader.header = Some(VbcHeader { version_minor: 27, ..VbcHeader::default() });
    let descriptor = reader.parse_function_descriptor().unwrap();
    assert_eq!(descriptor.return_type, TypeRef::Concrete(TypeId::UNIT));
    assert_eq!(descriptor.semantic_return, Some(TypeRef::Concrete(TypeId::INT)));
    assert_eq!(reader.offset, boundary);
    assert_eq!(reader.parse_function_descriptor().unwrap().id.0, NEXT_ID);
    assert_eq!(reader.offset, bytes.len());
}

#[test]
fn semantic_result_tail_refuses_malformed_evidence_instead_of_treating_it_as_absent() {
    let mut prefix = function_bytes(27);
    assert_eq!(prefix.pop(), Some(0));
    for tail in [vec![], vec![2], vec![1], vec![1, 255], vec![1, 1], vec![1, 1, 2, 0, 0]] {
        let mut bytes = prefix.clone();
        bytes.extend(tail);
        let mut reader = Deserializer::new(&bytes);
        reader.header = Some(VbcHeader { version_minor: 27, ..VbcHeader::default() });
        assert!(reader.parse_function_descriptor().is_err());
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

#[test]
fn semantic_formal_tail_distinguishes_legacy_absent_and_proved_empty() {
    for minor in [15, 16, 20, 22, 23] {
        let bytes = function_bytes(minor);
        let mut reader = Deserializer::new(&bytes);
        reader.header = Some(VbcHeader { version_minor: minor, ..VbcHeader::default() });
        assert!(reader.parse_function_descriptor().unwrap().semantic_params.is_none());
    }
    let mut bytes = function_bytes(23);
    bytes.pop();
    bytes.extend([1, 0]); // present, exact zero declaration slots
    append_next_descriptor(&mut bytes, 23);
    let mut reader = Deserializer::new(&bytes);
    reader.header = Some(VbcHeader { version_minor: 23, ..VbcHeader::default() });
    assert_eq!(reader.parse_function_descriptor().unwrap().semantic_parameter_types(), Some([].as_slice()));
    assert_eq!(reader.offset, 45);
    assert_eq!(reader.parse_function_descriptor().unwrap().id.0, NEXT_ID);
}

#[test]
fn semantic_formal_tail_rejects_invalid_presence_arity_and_bounds() {
    let mut prefix = function_bytes(23);
    prefix.pop();
    for tail in [vec![], vec![2], vec![1], vec![1, 1], vec![1, 2, 0, 0]] {
        let mut bytes = prefix.clone();
        bytes.extend(tail);
        let mut reader = Deserializer::new(&bytes);
        reader.header = Some(VbcHeader { version_minor: 23, ..VbcHeader::default() });
        assert!(reader.parse_function_descriptor().is_err());
    }
    prefix.push(1);
    crate::encoding::encode_varint((MAX_FN_TYPE_REF_PARAMS + 1) as u64, &mut prefix);
    let mut reader = Deserializer::new(&prefix);
    reader.header = Some(VbcHeader { version_minor: 23, ..VbcHeader::default() });
    assert!(matches!(reader.parse_function_descriptor(), Err(VbcError::TableTooLarge { field: "fn_semantic_param_count", .. })));
}

#[test]
fn semantic_formal_slot_has_independent_presence_and_type_boundaries() {
    // The frozen v2.15 descriptor with one Unit ABI parameter. Its 11 bytes
    // are name:u32, Concrete(Unit):5 bytes, is_mut:u8 and default:none:u8.
    let mut prefix = V215_FUNCTION.to_vec();
    prefix[28] = 1; // ordinary parameter count
    prefix[39] = 1; // optional source-spelling count
    prefix.insert(40, 0); // absent spelling, before origin
    prefix.splice(29..29, [0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    prefix.extend([0, 0]); // v16/v20 tails
    assert_eq!(prefix.len(), 55);
    for tail in [vec![1, 1], vec![1, 1, 2], vec![1, 1, 1], vec![1, 1, 1, 255]] {
        let mut bytes = prefix.clone();
        bytes.extend(tail);
        let mut reader = Deserializer::new(&bytes);
        reader.header = Some(VbcHeader { version_minor: 23, ..VbcHeader::default() });
        assert!(reader.parse_function_descriptor().is_err());
    }
    for (tail, expected) in [(vec![1, 1, 0], None), (vec![1, 1, 1, 1, 2, 0, 0, 0], Some(TypeRef::Concrete(TypeId::INT)))] {
        let mut bytes = prefix.clone();
        bytes.extend(tail);
        let boundary = bytes.len();
        append_next_descriptor(&mut bytes, 23);
        let mut reader = Deserializer::new(&bytes);
        reader.header = Some(VbcHeader { version_minor: 23, ..VbcHeader::default() });
        let first = reader.parse_function_descriptor().unwrap();
        assert_eq!(first.params[0].type_ref, TypeRef::Concrete(TypeId::UNIT));
        assert_eq!(first.semantic_parameter_types().unwrap(), &[expected]);
        assert_eq!(reader.offset, boundary);
        assert_eq!(reader.parse_function_descriptor().unwrap().id.0, NEXT_ID);
    }
}

#[test]
fn v243_callable_identity_preserves_the_next_descriptor_boundary() {
    for parent in [None, Some(FunctionId(0x44332211))] {
        let mut bytes = function_bytes(42);
        bytes.push(1); // declaration identity present
        bytes.extend_from_slice(&0x10203040u32.to_le_bytes());
        bytes.push(u8::from(parent.is_some()));
        if let Some(parent) = parent {
            bytes.extend_from_slice(&parent.0.to_le_bytes());
        }
        let boundary = bytes.len();
        append_next_descriptor(&mut bytes, 43);
        let mut reader = Deserializer::new(&bytes);
        reader.header = Some(VbcHeader { version_minor: 43, ..VbcHeader::default() });
        let first = reader.parse_function_descriptor().unwrap();
        assert_eq!(first.id.0, FIRST_ID);
        let identity = first.declaration_identity.expect("present identity");
        assert_eq!(identity.source_name, StringId(0x10203040));
        assert_eq!(identity.lexical_parent, parent);
        assert_eq!(reader.offset, boundary);
        let second = reader.parse_function_descriptor().unwrap();
        assert_eq!(second.id.0, NEXT_ID);
        assert!(second.declaration_identity.is_none());
        assert_eq!(reader.offset, bytes.len());
    }
}

#[test]
fn v243_callable_identity_rejects_invalid_tags_and_truncation() {
    let prefix = function_bytes(42);
    let present = [1, 0x40, 0x30, 0x20, 0x10, 1, 0x11, 0x22, 0x33, 0x44];
    for tail in (0..present.len()).map(|len| &present[..len])
        .chain([&[2][..], &[1, 0x40, 0x30, 0x20, 0x10, 2][..]])
    {
        let bytes = [prefix.as_slice(), tail].concat();
        let mut reader = Deserializer::new(&bytes);
        reader.header = Some(VbcHeader { version_minor: 43, ..VbcHeader::default() });
        assert!(reader.parse_function_descriptor().is_err(), "tail={tail:?}");
    }
}
