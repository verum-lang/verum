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

#[test]
fn packed_element_keeps_its_address_at_the_darwin_write_boundary() {
    let module = fixture::module_with_client(r#"
        @ffi("libSystem.B.dylib")
        extern "C" { fn write(fd: Int32, data: &Byte, count: Int) -> Int; }

        fn middle(values: &mut List<Byte>) -> &mut Byte { &mut values[1] }

        fn emit(value: &mut Byte) -> Int {
            *value = 97 as Byte;
            write(1 as Int32, value, 1)
        }

        fn main() -> Int {
            let mut packed: List<Byte> = List<Byte>.with_capacity(3);
            packed.push(65 as Byte);
            packed.push(66 as Byte);
            packed.push(67 as Byte);
            let count = emit(middle(&mut packed));
            if count == 1 && packed[0] == 65 as Byte && packed[1] == 97 as Byte &&
                packed[2] == 67 as Byte { 73 } else { 1 }
        }
    "#);
    let output = source_image::execute_module(module, |_| {});
    assert_eq!(output.status.code(), Some(73));
    assert_eq!(output.stdout, b"a");
}

#[test]
fn packed_exchange_rejects_a_word_outside_the_byte_range() {
    let module = fixture::module_with_client(r#"
        fn exchange(values: &mut List<Byte>, wide: &mut Int) {
            @intrinsic("swap", wide, &mut values[1]);
        }
        fn main() -> Int {
            let mut values: List<Byte> = List<Byte>.with_capacity(3);
            values.push(11 as Byte);
            values.push(99 as Byte);
            values.push(44 as Byte);
            let mut wide = 300;
            let refused = try { exchange(&mut values, &mut wide); 0 } recover |_| { 1 };
            if refused != 1 || wide != 300 || values[1] != 99 as Byte { return 2; }
            wide = 42;
            exchange(&mut values, &mut wide);
            if values[0] == 11 as Byte && values[1] == 42 as Byte &&
                values[2] == 44 as Byte && wide == 99 { 73 } else { 3 }
        }
    "#);
    source_image::run_module(module, |_| {});
}

#[test]
fn both_legacy_element_encodings_execute_natively_from_vbc_2_69() {
    for annotated in [false, true] {
        source_image::run_module(fixture::legacy_read(annotated), |_| {});
    }
}
