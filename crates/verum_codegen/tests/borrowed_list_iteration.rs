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

#[test]
fn packed_byte_constructor_uses_the_declared_storage_allocator() {
    let module = fixture::module_with_client(r#"
        fn main() -> Int {
            let mut packed: List<Byte> = List<Byte>.with_capacity(3);
            packed.push(21 as Byte);
            packed.push(42 as Byte);
            let ordinary: List<Byte> = [21 as Byte, 42 as Byte];
            if @intrinsic("list_storage_stride", packed) == 1 &&
                @intrinsic("list_storage_stride", ordinary) == 8 &&
                packed.len() == 2 && ordinary[0] == packed[0] &&
                ordinary[1] == packed[1] { 73 } else { 1 }
        }
    "#);
    source_image::run_module(module, |_| {});
}
