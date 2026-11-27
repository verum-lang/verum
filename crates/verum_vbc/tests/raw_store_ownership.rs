//! A raw write receives its payload before the source wrapper leaves scope.
#![cfg(feature = "codegen")]

#[path = "fixtures/canonical_drop.rs"]
mod canonical_drop;

use verum_fast_parser::Parser;
use verum_vbc::{Value, codegen::VbcCodegen, interpreter::Interpreter};

#[test]
fn raw_stores_receive_owned_values_before_the_source_wrapper_leaves_scope() {
    let source = r#"
        mount core.base.protocols.Drop;
        static mut RELEASED: Int = 0;
        type affine Ticket is { number: Int };
        type Plain is { number: Int };
        implement Drop for Ticket {
            fn drop(&mut self) { unsafe { RELEASED = RELEASED + self.number; } }
        }
        fn store<T>(address: *mut T, value: T) {
            unsafe { @intrinsic("ptr_write", address, value); }
        }
        fn store_volatile<T>(address: *mut T, value: T) {
            unsafe { @intrinsic("ptr_write_volatile", address, value); }
        }
        fn write(address: *mut Ticket) { store(address, Ticket { number: 73 }); }
        fn write_volatile(address: *mut Ticket) { store_volatile(address, Ticket { number: 73 }); }
        fn copy_plain(address: *mut Plain) -> Int {
            let mut original = Plain { number: 13 };
            store<Plain>(address, original);
            original.number = 29;
            original.number
        }
        fn copy_plain_volatile(address: *mut Plain) -> Int {
            let mut original = Plain { number: 13 };
            store_volatile<Plain>(address, original);
            original.number = 29;
            original.number
        }
        fn read_plain(payload: Plain) -> Int { payload.number }
        fn finish(ticket: Ticket) -> Int { ticket.number }
        fn releases() -> Int { unsafe { RELEASED } }
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let mut generator = VbcCodegen::new();
    canonical_drop::initialize(&mut generator);
    let module = generator.compile_additional_module(&ast).unwrap();
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap();
    for image in [module, wire] {
        let finish = image.find_function_by_name("finish").unwrap();
        let releases = image.find_function_by_name("releases").unwrap();
        for name in ["write", "write_volatile"] {
            let write = image.find_function_by_name(name).unwrap();
            let mut vm = Interpreter::try_new_validated(image.clone().into()).unwrap();
            vm.run_global_ctors().unwrap();
            let mut word = 0_u64;
            let address = Value::from_ptr((&mut word as *mut u64).cast::<u8>());
            vm.execute_function_with_args(write, &[address]).unwrap();
            assert_eq!(
                vm.execute_function(releases).unwrap().try_as_i64(),
                Some(0),
                "{name}"
            );
            assert_ne!(word, 0);
            let payload = Value::from_ptr(word as *mut u8);
            assert_eq!(
                vm.execute_function_with_args(finish, &[payload])
                    .unwrap()
                    .try_as_i64(),
                Some(73)
            );
            assert_eq!(
                vm.execute_function(releases).unwrap().try_as_i64(),
                Some(73),
                "{name}"
            );
        }
        for name in ["copy_plain", "copy_plain_volatile"] {
            let store = image.find_function_by_name(name).unwrap();
            let inspect = image.find_function_by_name("read_plain").unwrap();
            let mut vm = Interpreter::try_new_validated(image.clone().into()).unwrap();
            let mut word = 0_u64;
            let address = Value::from_ptr((&mut word as *mut u64).cast::<u8>());
            assert_eq!(vm.call(store, &[address]).unwrap().try_as_i64(), Some(29));
            assert_ne!(word, 0);
            let stored = Value::from_ptr(word as *mut u8);
            assert_eq!(
                vm.call(inspect, &[stored]).unwrap().try_as_i64(),
                Some(13),
                "{name}"
            );
        }
    }
}

#[test]
fn encoded_raw_stores_write_only_the_selected_machine_width() {
    use verum_vbc::{
        FunctionId, Instruction, Reg, VbcModule,
        bytecode::encode_instruction,
        instruction::MemSubOpcode,
        module::{FunctionDescriptor, ParamDescriptor},
    };

    for operation in [MemSubOpcode::PtrWrite, MemSubOpcode::PtrWriteVolatile] {
        for width in [1, 2, 4, 8] {
            let mut module = VbcModule::new("raw_store_width".into());
            let name = module.intern_string("write");
            for op in [
                Instruction::MemExtended {
                    sub_op: operation as u8,
                    operands: vec![0, 1, width],
                },
                Instruction::Ret { value: Reg(1) },
            ] {
                encode_instruction(&op, &mut module.bytecode);
            }
            let entry = module.add_function(FunctionDescriptor {
                id: FunctionId(0),
                name,
                params: (0..2).map(|_| ParamDescriptor::default()).collect(),
                register_count: 2,
                bytecode_length: module.bytecode.len() as u32,
                ..Default::default()
            });
            module.header.function_table_count = module.functions.len() as u32;
            let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
            let wire = verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap();
            for image in [module, wire] {
                let mut vm = Interpreter::try_new_validated(image.into()).unwrap();
                for (value, raw) in [
                    (Value::from_i64(0x1234_5678), 0x1234_5678_u64),
                    (Value::from_bool(true), 1),
                    (Value::from_bool(false), 0),
                    (Value::from_f64(3.25), 3.25_f64.to_bits()),
                ] {
                    let mut buf = [0x4e4e_4e4e_4e4e_4e4e_u64; 2];
                    let address = Value::from_ptr(buf.as_mut_ptr().cast::<u8>());
                    vm.call(entry, &[address, value]).unwrap();
                    let initial = 0x4e4e_4e4e_4e4e_4e4e_u64.to_ne_bytes();
                    let mut expected = initial;
                    expected[..usize::from(width)]
                        .copy_from_slice(&raw.to_ne_bytes()[..usize::from(width)]);
                    assert_eq!(buf[0].to_ne_bytes(), expected, "{operation:?}, {width}");
                    assert_eq!(buf[1].to_ne_bytes(), initial);
                }
            }
        }
    }
}
