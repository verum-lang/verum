//! Analysis controls only: no native ABI or foreign function is executed here.
use verum_common::List;
use verum_vbc::instruction::{Instruction, Reg, RegRange};
use verum_vbc::module::{FunctionDescriptor, FunctionId, VbcModule};
use verum_vbc::reachability;
use verum_vbc::types::{TypeDescriptor, TypeId, TypeKind, TypeRef};

const RECORD: TypeId = TypeId(700);

fn call(id: u32) -> Instruction {
    Instruction::Call {
        dst: Reg(0),
        func_id: id,
        args: RegRange { start: Reg(0), count: 0 },
    }
}

fn fixture() -> VbcModule {
    let mut module = VbcModule::new("materialized_receiver".into());
    for (name, instructions) in [
        ("main", List::new()),
        ("foreign.Gadget.tick", List::from_iter([call(3)])),
        ("foreign.make_gadget", List::new()),
        ("method_dependency", List::new()),
        ("foreign.Gadget.uncalled", List::new()),
        ("dead_function", List::new()),
    ] {
        let mut function = FunctionDescriptor::new(module.intern_string(name));
        function.id = FunctionId(module.functions.len() as u32);
        function.register_count = 1;
        function.instructions = Some(instructions.into());
        module.functions.push(function);
    }
    let mut descriptor = TypeDescriptor::default();
    descriptor.id = RECORD;
    descriptor.name = module.intern_string("foreign.Gadget");
    descriptor.kind = TypeKind::Record;
    module.types.push(descriptor);
    module.functions[1].parent_type = Some(RECORD);
    module.functions[1].source_receiver = Some(TypeRef::Concrete(RECORD));
    module.functions[4].parent_type = Some(RECORD);
    let method = module.intern_string("tick");
    module.functions[0].instructions = Some(List::from_iter([Instruction::CallM {
        dst: Reg(0),
        receiver: Reg(0),
        method_id: method.0,
        args: RegRange { start: Reg(0), count: 0 },
    }]).into());
    module
}

fn assert_callm_closure(module: &VbcModule) {
    let reached = reachability::analyze(module).unwrap();
    assert!(reached.reachable_ids.contains(&1), "record method must remain a CallM candidate");
    assert!(reached.reachable_ids.contains(&3), "candidate body dependencies must be followed");
    assert!(!reached.reachable_ids.contains(&4), "a method with an uncalled name is not a root");
    assert!(!reached.reachable_ids.contains(&5), "an unrelated function is not a root");
    assert!(!reached.reachable_type_ids.contains(&RECORD.0), "this fixture has no local construction");
}

#[test]
fn conservative_callm_keeps_never_constructed_record_methods() {
    // Restore the original T0392 oracle through today's analysis entry point.
    // The receiver is unknown to this function-level walk. An absent New is
    // not proof that a record cannot arrive through a parameter or foreign ABI.
    assert_callm_closure(&fixture());
}

#[test]
fn a_declared_record_return_does_not_require_local_construction() {
    let mut module = fixture();
    let producer = &mut module.functions[2];
    producer.instructions = None;
    producer.return_type = TypeRef::Concrete(RECORD);
    producer.semantic_return = Some(TypeRef::Concrete(RECORD));
    module.functions[0].instructions.as_mut().unwrap().insert(0, call(2));
    assert_callm_closure(&module);
    assert!(reachability::analyze(&module).unwrap().reachable_ids.contains(&2));
}
