#[path = "../support/selected_stdlib.rs"]
mod selected_stdlib;

pub fn module() -> verum_vbc::VbcModule {
    use verum_ast::FileId;
    use verum_fast_parser::FastParser;
    use verum_vbc::codegen::{ItemFailurePolicy, VbcCodegen};

    let selection = [
        ("mod.vr", ""),
        ("base/mod.vr", "Result Maybe"),
        ("base/maybe.vr", "Maybe Maybe<T>.is_some"),
        ("base/result.vr", "Result"),
        (
            "sys/common.vr",
            "SysContextError CONTEXT_STACK_DEPTH ctx_get ctx_set ctx_clear ctx_has ctx_push_frame ctx_pop_frame __ctx_slot_get_pointer __ctx_slot_set_pointer __ctx_slot_clear_raw __ctx_frame_push_raw __ctx_frame_pop_raw",
        ),
    ];
    let mut units: verum_common::List<_> = selection
        .iter()
        .enumerate()
        .map(|(index, (path, names))| selected_stdlib::source_unit(path, names, index))
        .collect();
    let test = FastParser::new()
        .parse_module_str(include_str!("context_slots.vr"), FileId::new(10))
        .unwrap();
    units.last_mut().unwrap().items.extend(test.items);
    let sources: verum_common::List<_> = units.iter().collect();
    let mut generator = VbcCodegen::new();
    generator.initialize();
    generator.collect_unit_declarations(&sources).unwrap();
    generator.resolve_pending_imports();
    generator
        .compile_unit_items(&sources, ItemFailurePolicy::Strict)
        .unwrap();
    let mut module = generator.finalize_module().unwrap();
    module.resolve_protocol_dispatch();
    module
}

/// The named handler instructions predate the numeric slot operations.
pub fn legacy_module() -> verum_vbc::VbcModule {
    use verum_vbc::instruction::{Instruction, Reg};
    use verum_vbc::module::FunctionDescriptor;
    use verum_vbc::types::{TypeId, TypeRef};

    let mut module = verum_vbc::VbcModule::new("legacy_context".into());
    let other = module.intern_string("Unrelated").0;
    let key = module.intern_string("Counter").0;
    let mut function = FunctionDescriptor::new(module.intern_string("main"));
    function.return_type = TypeRef::Concrete(TypeId::INT);
    function.register_count = 2;
    let body = vec![
        Instruction::LoadI {
            dst: Reg(0),
            value: 73,
        },
        Instruction::PushContext {
            name: key,
            handler: Reg(0),
        },
        Instruction::LoadI {
            dst: Reg(0),
            value: 9,
        },
        Instruction::PushContext {
            name: other,
            handler: Reg(0),
        },
        Instruction::PushContext {
            name: key,
            handler: Reg(0),
        },
        Instruction::PopContext { name: other },
        Instruction::PopContext { name: key },
        Instruction::CtxGet {
            dst: Reg(1),
            ctx_type: key,
        },
        Instruction::PopContext { name: key },
        Instruction::Ret { value: Reg(1) },
    ];
    verum_vbc::bytecode::encode_instructions(&body, &mut module.bytecode);
    function.bytecode_length = module.bytecode.len() as u32;
    function.instructions = Some(body);
    module.add_function(function);
    module.header.function_table_count = 1;
    module
}

/// The register order and operation numbers are from the VBC 2.71 wire format.
pub fn numeric_module() -> verum_vbc::VbcModule {
    use verum_vbc::instruction::{Instruction, Reg};
    let mut module = legacy_module();
    module.header.version_minor = 71;
    let body = vec![
        Instruction::LoadI { dst: Reg(0), value: 18 },
        Instruction::LoadI { dst: Reg(1), value: 73 },
        Instruction::SysExtended { sub_op: 0x37, operands: vec![2, 0, 1] },
        Instruction::SysExtended { sub_op: 0x36, operands: vec![2, 0] },
        Instruction::Ret { value: Reg(2) },
    ];
    module.bytecode.clear();
    verum_vbc::bytecode::encode_instructions(&body, &mut module.bytecode);
    module.functions[0].register_count = 3;
    module.functions[0].instructions = Some(body);
    module.functions[0].bytecode_length = module.bytecode.len() as u32;
    module
}
