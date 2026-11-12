//! Source layouts, archived carriers and live VM allocations must agree at FFI.
#![cfg(all(feature = "codegen", feature = "ffi"))]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::VbcCodegen,
    ffi::runtime::{marshal_c_to_verum_struct_at, marshal_verum_struct_to_c},
    interpreter::Interpreter,
    module::VbcModule,
    types::TypeId,
};

const SOURCE: &str = r#"
    type Pair is { key: Int16, value: UInt64 };
    type Packet is {
        lead: UInt8, words: [Int32; 3], pairs: [Pair; 2],
        grid: [[UInt16; 2]; 2], tail: UInt64,
    };
    extern "C" { fn inspect_packet(packet: &unsafe Packet) -> Int32; }
    fn make_packet() -> Packet {
        Packet {
            lead: 7 as UInt8,
            words: [-7 as Int32, 23 as Int32, 99 as Int32],
            pairs: [Pair { key: -2 as Int16, value: 41 as UInt64 },
                    Pair { key: 5 as Int16, value: 73 as UInt64 }],
            grid: [[1 as UInt16, 2 as UInt16], [3 as UInt16, 4 as UInt16]],
            tail: 9007199254740993 as UInt64,
        }
    }
"#;

fn compile(source: &str) -> VbcModule {
    VbcCodegen::new()
        .compile_module(&Parser::new(source).parse_module().unwrap())
        .unwrap()
}

#[repr(C)]
struct NativePair {
    key: i16,
    value: u64,
}
#[repr(C)]
struct NativePacket {
    lead: u8,
    words: [i32; 3],
    pairs: [NativePair; 2],
    grid: [[u16; 2]; 2],
    tail: u64,
}

#[test]
fn archived_inline_arrays_use_live_element_storage_in_both_directions() {
    let source = compile(SOURCE);
    let wire = verum_vbc::serialize::serialize_module(&source).unwrap();
    let decoded = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
    for module in [source, decoded] {
        let layout = module
            .ffi_layouts
            .iter()
            .find(|layout| module.strings.get(layout.name) == Some("Packet"))
            .unwrap()
            .clone();
        assert_eq!(layout.size as usize, size_of::<NativePacket>());
        assert_eq!(layout.align as usize, align_of::<NativePacket>());
        for (field, offset) in layout.fields.iter().zip([
            std::mem::offset_of!(NativePacket, lead),
            std::mem::offset_of!(NativePacket, words),
            std::mem::offset_of!(NativePacket, pairs),
            std::mem::offset_of!(NativePacket, grid),
            std::mem::offset_of!(NativePacket, tail),
        ]) {
            assert_eq!(field.offset as usize, offset);
        }
        for index in [1, 2, 3] {
            let array = &module.ffi_layouts[layout.fields[index].nested_layout.unwrap() as usize];
            assert_eq!(array.verum_type, Some(TypeId::ARRAY));
        }
        let function = module.find_function_by_name("make_packet").unwrap();
        let module = Arc::new(module);
        let mut vm = Interpreter::new(module.clone());
        let object = vm.execute_function(function).unwrap();
        let mut bytes = [0u8; size_of::<NativePacket>()];
        // SAFETY: vm owns the complete aggregate until both transfers finish.
        unsafe {
            marshal_verum_struct_to_c(&layout, &module.ffi_layouts, object.as_ptr(), &mut bytes)
        }
        .unwrap();
        assert_eq!(bytes[0], 7);
        assert_eq!(i32::from_ne_bytes(bytes[4..8].try_into().unwrap()), -7);
        assert_eq!(i32::from_ne_bytes(bytes[8..12].try_into().unwrap()), 23);
        assert_eq!(i32::from_ne_bytes(bytes[12..16].try_into().unwrap()), 99);
        assert_eq!(i16::from_ne_bytes(bytes[16..18].try_into().unwrap()), -2);
        assert_eq!(u64::from_ne_bytes(bytes[24..32].try_into().unwrap()), 41);
        assert_eq!(i16::from_ne_bytes(bytes[32..34].try_into().unwrap()), 5);
        assert_eq!(u64::from_ne_bytes(bytes[40..48].try_into().unwrap()), 73);
        for (i, expected) in [1u16, 2, 3, 4].iter().enumerate() {
            assert_eq!(
                u16::from_ne_bytes(bytes[48 + i * 2..50 + i * 2].try_into().unwrap()),
                *expected
            );
        }
        assert_eq!(
            u64::from_ne_bytes(bytes[56..64].try_into().unwrap()),
            9007199254740993
        );
        bytes[8..12].copy_from_slice(&(-600i32).to_ne_bytes());
        bytes[40..48].copy_from_slice(&(u64::MAX - 7).to_ne_bytes());
        bytes[54..56].copy_from_slice(&65000u16.to_ne_bytes());
        // SAFETY: same live VM allocations and exact native buffer extent.
        unsafe {
            marshal_c_to_verum_struct_at(
                &layout,
                &module.ffi_layouts,
                bytes.as_ptr(),
                bytes.len(),
                object.as_ptr(),
                0,
                0,
            );
        }
        let mut roundtrip = [0u8; size_of::<NativePacket>()];
        // SAFETY: same live aggregate, untouched allocation ownership.
        unsafe {
            marshal_verum_struct_to_c(
                &layout,
                &module.ffi_layouts,
                object.as_ptr(),
                &mut roundtrip,
            )
        }
        .unwrap();
        assert_eq!(roundtrip, bytes);

        let mut invalid_layout = layout.clone();
        let mut invalid_children = module.ffi_layouts.clone();
        let words = &mut invalid_children[layout.fields[1].nested_layout.unwrap() as usize];
        words.fields.pop();
        words.size = 8;
        invalid_layout.fields[1].size = 8;
        let mut untouched = [0xa5u8; size_of::<NativePacket>()];
        // SAFETY: the allocation is live; only declared array extent is forged.
        assert!(
            unsafe {
                marshal_verum_struct_to_c(
                    &invalid_layout,
                    &invalid_children,
                    object.as_ptr(),
                    &mut untouched,
                )
            }
            .is_err()
        );
        assert!(untouched.iter().all(|byte| *byte == 0xa5));
    }
}

#[test]
fn aggregate_nesting_has_a_checked_bound_on_an_ordinary_stack() {
    for (depth, accepted) in [(6, true), (9, false)] {
        let mut ty = "UInt8".to_owned();
        for _ in 0..depth {
            ty = format!("[{ty}; 2]");
        }
        let source = format!(
            "type Packet is {{ data: {ty} }}; extern \"C\" {{ fn inspect_packet(p: &unsafe Packet); }}"
        );
        let ast = Parser::new(&source).parse_module().unwrap();
        let result = VbcCodegen::new().compile_module(&ast);
        assert_eq!(result.is_ok(), accepted, "depth {depth}: {result:?}");
    }
}

#[test]
fn invalid_array_extents_are_compile_errors() {
    for extent in ["0", "-1", "8192", "unresolved"] {
        let source = format!(
            "type Packet is {{ data: [Int64; {extent}] }}; extern \"C\" {{ fn inspect_packet(p: &unsafe Packet); }}"
        );
        let ast = Parser::new(&source).parse_module().unwrap();
        assert!(
            VbcCodegen::new().compile_module(&ast).is_err(),
            "accepted {extent}"
        );
    }
}
