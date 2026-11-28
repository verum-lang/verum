//! The linked image transfers a record and retires each nested owner once.
#![cfg(target_os = "macos")]
#[path = "../../verum_vbc/tests/fixtures/owned_flat_record.rs"]
mod fixture;
#[path = "support/source_image.rs"]
mod source_image;

#[test]
fn linked_records_reuse_raw_storage_after_consuming_the_old_owner() {
    let module = fixture::source_module();
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap();
    source_image::run_module(wire, |_| {});
}
