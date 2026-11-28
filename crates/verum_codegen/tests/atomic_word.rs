#![cfg(target_os = "macos")]
#[path = "../../verum_vbc/tests/fixtures/atomic_word.rs"]
mod atomic_word;
#[path = "support/source_image.rs"]
mod source_image;

#[test]
fn native_atomic_words_preserve_all_bits_through_the_source_api() {
    let bytes = verum_vbc::serialize::serialize_module(&atomic_word::module()).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap();
    source_image::run_module(wire, |_| {});
}
