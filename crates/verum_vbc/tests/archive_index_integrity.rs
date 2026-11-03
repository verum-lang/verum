use std::io::{Cursor, ErrorKind};

use verum_vbc::archive::{ArchiveBuilder, VbcArchive, read_archive, write_archive};
use verum_vbc::module::VbcModule;

fn library() -> VbcArchive {
    let mut builder = ArchiveBuilder::new();
    let module = VbcModule::new("unit".into());
    builder.add_module("α", &module, &[]).unwrap();
    builder.add_module("β", &module, &["α"]).unwrap();
    builder.finish()
}

fn encode(archive: &VbcArchive) -> Vec<u8> {
    let mut bytes = Vec::new();
    write_archive(archive, &mut bytes).unwrap();
    bytes
}

fn assert_refused(bytes: Vec<u8>, reason: &str) {
    let error = read_archive(Cursor::new(bytes)).expect_err("invalid index accepted");
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert!(error.to_string().contains(reason), "{error}");
}

#[test]
fn unicode_module_names_and_indexed_dependencies_roundtrip() {
    let decoded = read_archive(Cursor::new(encode(&library()))).unwrap();
    assert_eq!(decoded.module_names_ordered(), ["α", "β"]);
    assert_eq!(decoded.index[1].dependencies, [0]);
}

#[test]
fn an_index_can_refer_to_a_module_later_in_the_archive() {
    let mut archive = library();
    archive.index[1].dependencies.clear();
    archive.index[0].dependencies.push(1);
    let decoded = read_archive(Cursor::new(encode(&archive))).unwrap();
    assert_eq!(decoded.module_names_ordered(), ["β", "α"]);
}

#[test]
fn two_entries_cannot_claim_the_same_module() {
    let mut archive = library();
    archive.index[1].name = archive.index[0].name.clone();
    assert_refused(encode(&archive), "duplicate module");
}

#[test]
fn invalid_utf8_does_not_decode_as_a_different_module() {
    let archive = library();
    let mut bytes = encode(&archive);
    let first_name_start = archive.header.index_offset as usize + size_of::<u32>();
    bytes[first_name_start] = 0xff;
    assert_refused(bytes, "UTF-8");
}

#[test]
fn dependency_must_identify_an_entry_in_the_same_archive() {
    let mut archive = library();
    archive.index[1].dependencies[0] = archive.header.module_count;
    assert_refused(encode(&archive), "dependency index");
}
