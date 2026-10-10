use super::*;
use verum_common::List;

fn old_field() -> List<u8> {
    let mut bytes = List::new();
    bytes.extend_from_slice(&17_u32.to_le_bytes()); // name
    bytes.push(1); // concrete TypeRef
    bytes.extend_from_slice(&TypeId::INT.0.to_le_bytes());
    bytes.extend_from_slice(&8_u32.to_le_bytes()); // offset
    bytes.push(Visibility::Public as u8); // coarse public is not declaration evidence
    for _ in 0..3 {
        bytes.extend_from_slice(&0_u32.to_le_bytes());
    }
    bytes.push(0); // no semantic type
    bytes
}

#[test]
fn legacy_v224_fields_remain_unknown_without_consuming_the_next_field() {
    let one = old_field();
    let mut bytes = one.clone();
    bytes.extend_from_slice(one.as_slice());
    let mut reader = Deserializer::new(bytes.as_slice());
    reader.header = Some(VbcHeader {
        version_minor: 24,
        ..VbcHeader::default()
    });
    for offset in [one.len(), bytes.len()] {
        let field = reader.parse_field().unwrap();
        assert_eq!(field.visibility, Visibility::Public);
        assert_eq!(field.declared_visibility, None);
        assert_eq!(reader.offset, offset);
    }
}

#[test]
fn new_field_policy_rejects_invalid_tags_and_truncated_scopes() {
    for tail in [&[][..], &[8][..], &[255][..], &[5][..], &[5, 1, 2, 3][..]] {
        let mut bytes = old_field();
        bytes.extend_from_slice(tail);
        let mut reader = Deserializer::new(bytes.as_slice());
        reader.header = Some(VbcHeader {
            version_minor: 25,
            ..VbcHeader::default()
        });
        assert!(reader.parse_field().is_err(), "tail {tail:?}");
    }
}
