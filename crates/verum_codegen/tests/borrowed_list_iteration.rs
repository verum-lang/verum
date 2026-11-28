//! Lower and execute the regular List iteration declarations.
#![cfg(target_os = "macos")]
#[path = "../../verum_vbc/tests/fixtures/borrowed_list_iteration.rs"]
mod fixture;
#[path = "support/source_image.rs"]
mod source_image;

#[test]
fn native_borrowed_list_positions_survive_the_next_cursor_call() {
    let bytes = verum_vbc::serialize::serialize_module(&fixture::module()).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap();
    source_image::run_module(wire, |_| {});
}
