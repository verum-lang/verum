//! A guard returned by the source Mutex keeps a borrow of its owning field.
#![cfg(target_os = "macos")]
#[path = "../../verum_vbc/tests/fixtures/retained_mutex_receiver.rs"]
mod actual_mutex;
#[path = "support/source_image.rs"]
mod source_image;

#[test]
fn a_native_guard_releases_the_original_mutex_before_reacquiring_it() {
    let source = actual_mutex::module();
    let bytes = verum_vbc::serialize::serialize_module(&source).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap();
    source_image::run_module(wire, |_| {});
}
