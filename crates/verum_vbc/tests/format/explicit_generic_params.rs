use super::*;

fn function_bytes() -> Vec<u8> {
    let mut module = VbcModule::new("generic_wire".into());
    let name = module.intern_string("sample");
    module.functions.push(FunctionDescriptor::new(name));
    let bytes = crate::serialize::serialize_module(&module).unwrap();
    let mut reader = Deserializer::new(&bytes);
    let header = reader.parse_header().unwrap();
    reader.offset = header.function_table_offset as usize;
    reader.header = Some(header.clone());
    reader.parse_function_descriptor().unwrap();
    bytes[header.function_table_offset as usize..reader.offset].to_vec()
}

#[test]
fn v215_function_table_does_not_consume_the_next_descriptor() {
    let mut old = function_bytes();
    // V2.16 writes a one-byte zero count even for no explicit parameters.
    assert_eq!(old.pop(), Some(0));
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
    let mut bytes = function_bytes();
    assert_eq!(bytes.pop(), Some(0));
    for tail in [Vec::new(), vec![1], vec![1, 1], vec![1, 1, 0, 0, 1, 0]] {
        let mut malformed = bytes.clone();
        malformed.extend(tail);
        let mut reader = Deserializer::new(&malformed);
        reader.header = Some(VbcHeader::default());
        assert!(reader.parse_function_descriptor().is_err());
    }
    let mut malformed = bytes;
    crate::encoding::encode_varint((MAX_FN_TYPE_REF_PARAMS + 1) as u64, &mut malformed);
    let mut reader = Deserializer::new(&malformed);
    reader.header = Some(VbcHeader::default());
    assert!(matches!(
        reader.parse_function_descriptor(),
        Err(VbcError::TableTooLarge {
            field: "fn_explicit_type_param_count",
            ..
        })
    ));
}
