#[path = "../support/selected_stdlib.rs"]
mod selected_stdlib;

pub fn module() -> verum_vbc::VbcModule {
    module_with_client(include_str!("borrowed_list_iteration.vr"))
}

pub fn module_with_client(source: &str) -> verum_vbc::VbcModule {
    use verum_ast::FileId;
    use verum_fast_parser::FastParser;
    use verum_vbc::codegen::{ItemFailurePolicy, VbcCodegen};

    let selection = [
        ("base/maybe.vr", "Maybe"),
        (
            "base/iterator.vr",
            "Iterator Iterator#next Iterator#size_hint IntoIterator IntoIterator#into_iter DoubleEndedIterator DoubleEndedIterator#next_back ExactSizeIterator ExactSizeIterator#len",
        ),
        ("base/protocols.vr", "Drop"),
        ("base/mod.vr", "Maybe"),
        ("mod.vr", ""),
        ("intrinsics/memory.vr", "list_storage_read list_storage_resize list_storage_stride list_storage_write"),
        ("intrinsics/arithmetic.vr", "checked_mul"),
        ("base/primitives.vr", "Int.checked_mul"),
        ("base/memory.vr", "swap replace"),
        (
            "collections/list.vr",
            "List list_max_len List<T>.push List<T>.grow List<T>.next_cap List<T>.resize_buffer List<T>.len ListIter ListIterMut List<T>.iter_mut Iterator@ListIterMut<T>.next Iterator@ListIterMut<T>.size_hint IntoIterator@ListIterMut<T>.into_iter DoubleEndedIterator@ListIterMut<T>.next_back ExactSizeIterator@ListIterMut<T>.len Drop@List<T>.drop List<T>.clear List<T>.truncate List<T>.free_buffer List<T>.iter Iterator@ListIter<T>.next Iterator@ListIter<T>.size_hint IntoIterator@ListIter<T>.into_iter DoubleEndedIterator@ListIter<T>.next_back ExactSizeIterator@ListIter<T>.len",
        ),
    ];
    let mut units: verum_common::List<_> = selection
        .iter()
        .enumerate()
        .map(|(index, (path, symbols))| selected_stdlib::source_unit(path, symbols, index))
        .collect();
    let client = FastParser::new()
        .parse_module_str(source, FileId::new(100))
        .unwrap();
    units.last_mut().unwrap().items.extend(client.items);
    let inputs: verum_common::List<_> = units.iter().collect();
    let mut generator = VbcCodegen::new();
    generator.initialize();
    generator.collect_unit_declarations(&inputs).unwrap();
    generator.resolve_pending_imports();
    generator
        .compile_unit_items(&inputs, ItemFailurePolicy::Strict)
        .unwrap();
    let mut module = generator.finalize_module().unwrap();
    module.resolve_protocol_dispatch();
    module
}

/// An executable VBC 2.69 body with the original indexed-element ABI.
pub fn legacy_read(annotated: bool) -> verum_vbc::VbcModule {
    use verum_vbc::{
        Instruction as I, Reg, VbcModule, bytecode::encode_instructions,
        instruction::CbgrSubOpcode, module::FunctionDescriptor, types::{TypeId, TypeRef},
    };
    let mut module = VbcModule::new("legacy".into());
    let mut main = FunctionDescriptor::new(module.intern_string("main"));
    main.return_type = TypeRef::Concrete(TypeId::INT);
    main.register_count = 4;
    let mut operands = vec![2, 0, 1];
    if annotated { operands.push(1); }
    let instructions = [
        I::NewList { dst: Reg(0), capacity_hint: 2 },
        I::LoadI { dst: Reg(1), value: 36 },
        I::ListPush { list: Reg(0), val: Reg(1) },
        I::LoadI { dst: Reg(1), value: 73 },
        I::ListPush { list: Reg(0), val: Reg(1) },
        I::LoadI { dst: Reg(1), value: 1 },
        I::CbgrExtended { sub_op: CbgrSubOpcode::RefListElement.to_byte(), operands },
        I::Deref { dst: Reg(3), ref_reg: Reg(2) },
        I::Ret { value: Reg(3) },
    ];
    main.bytecode_length = encode_instructions(&instructions, &mut module.bytecode) as u32;
    module.add_function(main);
    let mut bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    bytes[6..8].copy_from_slice(&69u16.to_le_bytes());
    verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap()
}
