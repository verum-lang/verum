use super::*;

// Frozen v2.16 type descriptor (aa2582846). The v2.17 writer (204121137)
// appends one presence byte and, when present, two little-endian u64s.
const V216_RECORD: [u8; 28] = [
    0x44, 0x33, 0x22, 0x11, // type ID
    0x88, 0x77, 0x66, 0x55, // name ID
    1, 1, // Record, Public
    16, 0, 0, 0, 8, 0, 0, 0, // slot size and alignment
    0, 0, 0, // type parameters, fields, variants
    0, 0, 0, // no drop/clone function, no protocols
    0, 0, 0, 0, // no alias, transparent flag, alias spelling or origin
];

fn reader(bytes: &[u8], minor: u16) -> Deserializer<'_> {
    let mut reader = Deserializer::new(bytes);
    reader.header = Some(VbcHeader { version_minor: minor, ..VbcHeader::default() });
    reader
}

#[test]
fn v216_descriptors_remain_unknown_without_consuming_the_next_descriptor() {
    let mut next = V216_RECORD;
    next[..4].copy_from_slice(&0x1234_5678_u32.to_le_bytes());
    let bytes = [V216_RECORD, next].concat();
    let mut reader = reader(&bytes, 16);
    for (end, id) in [(28, 0x1122_3344), (56, 0x1234_5678)] {
        let record = reader.parse_type_descriptor().unwrap();
        assert_eq!(record.id, TypeId(id));
        assert_eq!((record.size, record.alignment), (16, 8));
        assert_eq!(record.declared_layout, None);
        assert_eq!(reader.offset, end);
    }
}

#[test]
fn v217_layout_tail_rejects_unknown_tags_and_truncation() {
    let mut bytes = V216_RECORD.to_vec();
    bytes.push(2);
    assert!(matches!(reader(&bytes, 17).parse_type_descriptor(),
        Err(VbcError::InvalidHeader { field: "declared_layout", .. })));

    *bytes.last_mut().unwrap() = 1;
    bytes.extend(64_u64.to_le_bytes());
    bytes.extend(32_u64.to_le_bytes());
    let boundary = bytes.len();
    for end in V216_RECORD.len()..boundary {
        assert!(reader(&bytes[..end], 17).parse_type_descriptor().is_err(), "end={end}");
    }

    bytes.extend(V216_RECORD);
    bytes.push(0); // next declaration has no known layout
    let mut reader = reader(&bytes, 17);
    let layout = reader.parse_type_descriptor().unwrap().declared_layout.unwrap();
    assert_eq!((layout.size, layout.alignment), (64, 32));
    assert_eq!(reader.offset, boundary);
    assert_eq!(reader.parse_type_descriptor().unwrap().declared_layout, None);
    assert_eq!(reader.offset, bytes.len());
}
