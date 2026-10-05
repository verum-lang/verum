use super::*;

fn function_bytes(minor: u16) -> Vec<u8> {
    let mut module = VbcModule::new("generic_wire".into());
    let name = module.intern_string("sample");
    module.functions.push(FunctionDescriptor::new(name));
    let bytes = crate::serialize::serialize_module(&module).unwrap();
    let mut reader = Deserializer::new(&bytes);
    let header = reader.parse_header().unwrap();
    reader.offset = header.function_table_offset as usize;
    // Select the historical descriptor prefix using its actual reader gate.
    // Later versions append independent tails (v20 value-use receipts), so
    // popping the current serializer's last byte no longer locates v16.
    let mut historical = header.clone();
    historical.version_minor = minor;
    reader.header = Some(historical);
    reader.parse_function_descriptor().unwrap();
    bytes[header.function_table_offset as usize..reader.offset].to_vec()
}

#[test]
fn v215_function_table_does_not_consume_the_next_descriptor() {
    let mut old = function_bytes(15);
    let one_len = old.len();
    old.extend_from_within(..);
    let mut reader = Deserializer::new(&old);
    let mut header = VbcHeader::default();
    header.version_minor = 15;
    reader.header = Some(header);
    assert!(
        reader
            .parse_function_descriptor()
            .unwrap()
            .explicit_type_param_ids
            .is_empty()
    );
    assert_eq!(reader.offset, one_len);
    assert!(
        reader
            .parse_function_descriptor()
            .unwrap()
            .explicit_type_param_ids
            .is_empty()
    );
    assert_eq!(reader.offset, old.len());
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
    for minor in [15, 16, 19, 20, crate::format::VERSION_MINOR] {
        let mut bytes = function_bytes(minor);
        let one_len = bytes.len();
        bytes.extend_from_within(..);
        let mut reader = Deserializer::new(&bytes);
        reader.header = Some(VbcHeader { version_minor: minor, ..VbcHeader::default() });
        for expected in [one_len, bytes.len()] {
            let descriptor = reader.parse_function_descriptor().unwrap();
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
