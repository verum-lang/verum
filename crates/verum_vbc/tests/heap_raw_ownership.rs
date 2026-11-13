//! Statically and dynamically dispatched raw transfers preserve the same live Heap cell.
use verum_vbc::{
    FunctionId, Instruction as I, Reg, RegRange, VbcModule,
    bytecode::encode_instruction,
    interpreter::Interpreter,
    module::{FunctionDescriptor, ParamDescriptor},
};

fn function(module: &mut VbcModule, name: &str, arity: usize, body: &[I]) -> FunctionId {
    let name = module.intern_string(name);
    let offset = module.bytecode.len() as u32;
    for op in body {
        encode_instruction(op, &mut module.bytecode);
    }
    module.add_function(FunctionDescriptor {
        id: FunctionId(module.functions.len() as u32),
        name,
        params: (0..arity).map(|_| ParamDescriptor::default()).collect(),
        register_count: 3,
        bytecode_offset: offset,
        bytecode_length: module.bytecode.len() as u32 - offset,
        ..Default::default()
    })
}

fn method(module: &mut VbcModule, name: &str, arity: u8) -> FunctionId {
    let selector = module.intern_string(name);
    function(
        module,
        &format!("dispatch_{name}"),
        usize::from(arity) + 1,
        &[
            I::CallM {
                dst: Reg(2),
                receiver: Reg(0),
                method_id: selector.0,
                args: RegRange::new(Reg(1), arity),
            },
            I::Ret { value: Reg(2) },
        ],
    )
}

#[test]
fn only_the_declared_heap_transfers_its_cell() {
    use verum_vbc::{
        Value,
        interpreter::{CbgrViolationKind, InterpreterError},
        types::{TypeId, TypeRef},
    };
    let mut module = VbcModule::new("raw-transfer".into());
    let allocate = method(&mut module, "Heap.new", 1);
    let into = method(&mut module, "core.base.memory.Heap.into_raw", 0);
    let from = method(&mut module, "core.base.memory.Heap.from_raw", 1);
    let consume = method(&mut module, "Heap.into_inner", 0);
    let destroy = function(
        &mut module,
        "drop_cell",
        1,
        &[I::DropRef { src: Reg(0) }, I::Ret { value: Reg(0) }],
    );
    let operations: verum_common::List<_> = [
        "core.base.memory.Heap.into_raw",
        "core.base.memory.Heap.from_raw",
        "foreign.Heap.from_raw",
    ]
    .iter()
    .enumerate()
    .flat_map(|(index, name)| {
        let target = function(
            &mut module,
            name,
            1,
            &[
                I::LoadI {
                    dst: Reg(1),
                    value: 41,
                },
                I::Ret { value: Reg(1) },
            ],
        );
        let args = RegRange::new(Reg(0), 1);
        let dst = Reg(1);
        let func_id = target.0;
        let calls = [
            I::Call { dst, func_id, args },
            I::CallG {
                dst,
                func_id,
                type_args: vec![TypeRef::Concrete(TypeId::I64)],
                args,
            },
        ];
        calls
            .into_iter()
            .enumerate()
            .map(|(kind, call)| {
                let wrapper = function(
                    &mut module,
                    &format!("call_{kind}_{name}"),
                    1,
                    &[call, I::Ret { value: dst }],
                );
                (wrapper, index < 2)
            })
            .collect::<verum_common::List<_>>()
    })
    .collect();
    module.header.function_table_count = module.functions.len() as u32;
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    for module in [
        module,
        verum_vbc::deserialize::deserialize_module(&wire).unwrap(),
    ] {
        verum_vbc::validate::validate_module(&module).unwrap();
        let mut vm = Interpreter::new(module.into());
        let owner = Value::from_small_string("Heap").unwrap();
        let cell = vm.call(allocate, &[owner, Value::from_i64(73)]).unwrap();
        let raw = vm.call(into, &[cell]).unwrap();
        assert_eq!(raw, cell);
        for &(function, canonical) in &operations {
            let result = vm.call(function, &[raw]).unwrap();
            assert_eq!(result, if canonical { raw } else { Value::from_i64(41) });
            assert_eq!(
                vm.call(function, &[Value::from_i64(23)])
                    .unwrap()
                    .try_as_i64(),
                Some(41)
            );
        }
        let restored = vm.call(from, &[owner, raw]).unwrap();
        assert_eq!(
            vm.call(consume, &[restored]).unwrap().try_as_i64(),
            Some(73)
        );
        let dead = vm.call(allocate, &[owner, Value::from_i64(73)]).unwrap();
        vm.call(destroy, &[dead]).unwrap();
        let error = vm.call(from, &[owner, dead]).unwrap_err();
        assert!(
            matches!(error, InterpreterError::CbgrPanic { failure, .. } if failure.kind == CbgrViolationKind::UseAfterFree)
        );
    }
}
