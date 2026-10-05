use super::*;

fn descriptor_bytes() -> Vec<u8> {
    let mut module = VbcModule::new("layout_wire".into());
    let mut descriptor = TypeDescriptor::default();
    descriptor.id = TypeId(2048);
    descriptor.name = module.intern_string("Cell");
    module.types.push(descriptor);
    let bytes = crate::serialize::serialize_module(&module).unwrap();
    let mut reader = Deserializer::new(&bytes);
    let header = reader.parse_header().unwrap();
    reader.offset = header.type_table_offset as usize;
    reader.header = Some(header.clone());
    reader.parse_type_descriptor().unwrap();
    let mut descriptor = bytes[header.type_table_offset as usize..reader.offset].to_vec();
    // This fixture exercises the v2.17 tail; remove v2.18 resource discipline.
    assert_eq!(descriptor.pop(), Some(0));
    descriptor
}

#[test]
fn v216_descriptors_remain_unknown_without_consuming_the_next_descriptor() {
    let mut old = descriptor_bytes();
    assert_eq!(old.pop(), Some(0)); // v2.17 absent declaration fact.
    let one_len = old.len();
    old.extend_from_within(..);
    let mut reader = Deserializer::new(&old);
    let mut header = VbcHeader::default();
    header.version_minor = 16;
    reader.header = Some(header);
    assert_eq!(
        reader.parse_type_descriptor().unwrap().declared_layout,
        None
    );
    assert_eq!(reader.offset, one_len);
    assert_eq!(
        reader.parse_type_descriptor().unwrap().declared_layout,
        None
    );
    assert_eq!(reader.offset, old.len());
}

#[test]
fn v217_layout_tail_rejects_unknown_tags_and_truncation() {
    let mut prefix = descriptor_bytes();
    assert_eq!(prefix.pop(), Some(0));
    for tail in [vec![], vec![2], vec![1], vec![1, 8, 0, 0, 0, 0, 0, 0, 0]] {
        let mut malformed = prefix.clone();
        malformed.extend(tail);
        let mut reader = Deserializer::new(&malformed);
        let mut header = VbcHeader::default();
        header.version_minor = 17;
        reader.header = Some(header);
        assert!(reader.parse_type_descriptor().is_err());
    }
}
