#![cfg(target_os = "macos")]
#[path = "../../verum_vbc/tests/fixtures/maybe_replace_owner.rs"]
mod fixture;
#[path = "support/source_image.rs"]
mod source_image;

#[test]
fn native_maybe_replace_and_take_transfer_each_owner_once() {
    let bytes = verum_vbc::serialize::serialize_module(&fixture::module()).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap();
    source_image::run_module(wire, |_| {});
}
