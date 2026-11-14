//! Dynamic declarations retain semantic identity across nested carriers and the wire.
#![cfg(feature = "codegen")]

use verum_common::{List, ResourceDiscipline};
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::VbcCodegen,
    deserialize::deserialize_module,
    serialize::serialize_module,
    types::{TypeId, TypeRef},
    validate::validate_module,
};

fn compile(source: &str) -> VbcModule {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    VbcCodegen::new()
        .compile_module(&ast)
        .expect("declared dynamic source")
}

#[test]
fn dynamic_borrow_retains_a_semantic_formal_through_wire() {
    let module = compile(
        "type Policy is protocol {fn read(&self)->Int;};
         fn inspect(value:&dyn Policy)->Int {73}",
    );
    let decoded = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [&module, &decoded] {
        validate_module(module).unwrap();
        let function = module
            .functions
            .iter()
            .find(|function| module.get_string(function.name) == Some("inspect"))
            .unwrap();
        let declared = function.semantic_parameter_types().unwrap()[0]
            .as_ref()
            .expect("the selected dynamic declaration has a semantic witness");
        let TypeRef::Reference { inner, .. } = declared else {
            panic!("{declared:?}")
        };
        assert_ne!(inner.as_ref(), &TypeRef::Concrete(TypeId::PTR));
        assert!(
            !declared.is_generic(),
            "a closed object introduces no free caller slots"
        );
    }
}

#[test]
fn nested_dynamic_carrier_does_not_fabricate_caller_parameters() {
    let module = compile(
        "type Policy is protocol {fn read(&self)->Int;};
         type Carrier<T> is {value:Int};
         implement<T> Carrier<T> {fn new()->Self {Carrier {value:73}}}
         type Holder<T> is {value:T};
         implement<T> Holder<T> {fn new(value:T)->Self {Holder {value:value}}}
         type Registry is {item:Holder<Carrier<dyn Policy>>};
         fn create()->Registry {Registry {item:Holder.new(Carrier.new())}}",
    );
    let decoded = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    validate_module(&decoded).expect("all callable witness IDs belong to their declaration");
}

#[test]
fn unresolved_or_nonprotocol_bounds_never_acquire_pointer_authority() {
    for source in [
        "fn inspect(value:&dyn Missing)->Int {73}",
        "type Policy is {value:Int}; fn inspect(value:&dyn Policy)->Int {73}",
    ] {
        let ast = Parser::new(source).parse_module().expect("source grammar");
        let result = VbcCodegen::new().compile_module(&ast);
        assert!(result.is_err(), "{source}: erased declaration was accepted");
    }
}

#[test]
fn undeclared_associated_member_does_not_become_a_dynamic_witness() {
    let source = "type Policy is protocol {fn read(&self)->Int;};
        fn inspect(value:&dyn Policy<Missing=Int>)->Int {73}";
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let result = VbcCodegen::new().compile_module(&ast);
    assert!(result.is_err(), "undeclared associated member was accepted");
}

fn parameter(index: u16) -> TypeRef {
    TypeRef::Generic(verum_vbc::types::TypeParamId(index))
}

fn object(argument: TypeRef, equality: TypeRef) -> TypeRef {
    use verum_vbc::types::{
        DynamicAssociatedBinding, DynamicProtocolType, ProtocolApplication, ProtocolId,
    };
    let owner = ProtocolApplication {
        protocol: ProtocolId(3000),
        args: vec![argument],
    };
    TypeRef::Dynamic(DynamicProtocolType {
        bounds: List::from_iter([owner.clone()]),
        bindings: List::from_iter([DynamicAssociatedBinding {
            owner,
            member: "Item".into(),
            value: equality,
        }]),
    })
}

#[test]
fn existential_subject_cannot_capture_a_free_parameter_replacement() {
    use verum_vbc::{mono::TypeSubstitution, types::TypeParamId};
    let input = object(
        parameter(0),
        TypeRef::Tuple(vec![parameter(0), parameter(1)]),
    );
    let mut substitution = TypeSubstitution::empty();
    substitution.bind(TypeParamId(0), parameter(4));
    assert_eq!(
        substitution.apply(&input).unwrap(),
        object(
            parameter(0),
            TypeRef::Tuple(vec![parameter(0), parameter(5)])
        )
    );
    assert!(!object(parameter(0), parameter(0)).is_generic());
    assert!(input.is_generic());
}

#[test]
fn nested_rank2_and_existential_binders_keep_distinct_lexical_subjects() {
    use verum_vbc::{mono::TypeSubstitution, types::TypeParamId};
    let wrap = |inner| TypeRef::Rank2Function {
        is_async: false,
        type_param_count: 1,
        params: vec![parameter(0)],
        return_type: Box::new(inner),
        contexts: Default::default(),
    };
    let input = wrap(object(
        parameter(0),
        TypeRef::Tuple(vec![parameter(1), parameter(2)]),
    ));
    let mut substitution = TypeSubstitution::empty();
    substitution.bind(TypeParamId(0), parameter(3));
    assert_eq!(
        substitution.apply(&input).unwrap(),
        wrap(object(
            parameter(0),
            TypeRef::Tuple(vec![parameter(1), parameter(5)])
        ))
    );
    assert!(!wrap(object(parameter(0), parameter(1))).is_generic());
}

#[test]
fn dynamic_contract_roundtrips_in_instruction_and_archive() {
    use verum_vbc::{
        Instruction,
        bytecode::{decode_instruction, encode_instruction},
        instruction::Reg,
        module::FunctionDescriptor,
    };
    let ty = object(
        parameter(0),
        TypeRef::Tuple(vec![parameter(0), parameter(1)]),
    );
    let instruction = Instruction::LoadT {
        dst: Reg(0),
        type_ref: ty.clone(),
    };
    let mut bytes = Default::default();
    encode_instruction(&instruction, &mut bytes);
    let mut offset = 0;
    assert_eq!(
        decode_instruction(&bytes, &mut offset).unwrap(),
        instruction
    );
    assert_eq!(offset, bytes.len());
    let mut module = VbcModule::new("transport".into());
    let name = module.intern_string("contract");
    let mut function = FunctionDescriptor::new(name);
    function.return_type = ty.clone();
    module.functions.push(function);
    let decoded = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    assert_eq!(decoded.functions[0].return_type, ty);
}

#[test]
fn dynamic_relocation_retains_every_owner_and_does_not_rebind_subjects() {
    use verum_vbc::bytecode_remap::{IdRemap, rewrite_type_ref_ids};
    struct Relocate;
    impl IdRemap for Relocate {
        fn map_type_id(&self, id: TypeId) -> TypeId {
            TypeId(id.0 + 100)
        }
        fn map_protocol(&self, id: verum_vbc::types::ProtocolId) -> verum_vbc::types::ProtocolId {
            verum_vbc::types::ProtocolId(id.0 + 200)
        }
    }
    let mut ty = object(parameter(0), TypeRef::Concrete(TypeId(4000)));
    rewrite_type_ref_ids(&mut ty, &Relocate);
    let TypeRef::Dynamic(object) = ty else {
        panic!("dynamic type")
    };
    assert_eq!(object.bounds[0].protocol.0, 3200);
    assert_eq!(object.bounds[0].args, [parameter(0)]);
    assert_eq!(object.bindings[0].owner, object.bounds[0]);
    assert_eq!(object.bindings[0].value, TypeRef::Concrete(TypeId(4100)));
}

#[test]
fn source_defaults_and_explicit_self_share_the_existential_subject() {
    let module = compile(
        "type Policy<T=Self> is protocol {fn read(&self)->Int;};
        fn implicit(value:&dyn Policy)->Int {73}
        fn explicit(value:&dyn Policy<Self>)->Int {73}",
    );
    let declared = |name| {
        module
            .functions
            .iter()
            .find(|function| module.get_string(function.name) == Some(name))
            .unwrap()
            .semantic_parameter_types()
            .unwrap()[0]
            .clone()
            .unwrap()
    };
    assert_eq!(declared("implicit"), declared("explicit"));
    assert!(!declared("implicit").is_generic());
    validate_module(&module).unwrap();
}

#[test]
fn wire_rejects_forged_owners_members_and_escaped_subjects() {
    use verum_vbc::types::{
        DynamicAssociatedBinding, DynamicProtocolType, ProtocolApplication, ProtocolId,
    };
    let module = compile(
        "type Policy is protocol {type Item; fn read(&self)->Int;};
        type Other is protocol {type Item; fn read(&self)->Int;};
        type Record is {value:Int}; fn probe()->Int {73}",
    );
    let id = |name| {
        module
            .types
            .iter()
            .find(|ty| module.get_string(ty.name) == Some(name))
            .unwrap()
            .id
    };
    let application = |name| ProtocolApplication {
        protocol: ProtocolId(id(name).0),
        args: vec![],
    };
    let valid = DynamicProtocolType {
        bounds: List::from_iter([application("Policy")]),
        bindings: List::from_iter([DynamicAssociatedBinding {
            owner: application("Policy"),
            member: "Item".into(),
            value: TypeRef::Concrete(TypeId::INT),
        }]),
    };
    let check = |object| {
        let mut candidate = module.clone();
        candidate.functions[0].semantic_return = Some(TypeRef::Dynamic(object));
        let wire = serialize_module(&candidate).unwrap();
        let decoded = deserialize_module(&wire).unwrap();
        validate_module(&decoded)
    };
    check(valid.clone()).unwrap();
    assert_eq!(
        module.resource_discipline(&TypeRef::Dynamic(valid.clone())),
        ResourceDiscipline::Unknown
    );
    let mut wrong_kind = valid.clone();
    wrong_kind.bounds[0] = application("Record");
    assert!(check(wrong_kind).is_err());
    let mut foreign_owner = valid.clone();
    foreign_owner.bindings[0].owner = application("Other");
    assert!(check(foreign_owner).is_err());
    let mut unknown_member = valid.clone();
    unknown_member.bindings[0].member = "Missing".into();
    assert!(check(unknown_member).is_err());
    let mut escaped = valid;
    escaped.bindings[0].value = parameter(1);
    assert!(check(escaped).is_err());
}

#[test]
fn malformed_deep_dynamic_wire_is_rejected_on_the_ordinary_stack() {
    use verum_vbc::{
        Instruction,
        bytecode::{decode_instruction, encode_instruction},
        instruction::Reg,
    };
    let mut ty = TypeRef::Concrete(TypeId::INT);
    for _ in 0..verum_vbc::types::MAX_TYPE_REF_DEPTH + 1 {
        ty = object(parameter(0), ty);
    }
    let mut bytes = Default::default();
    encode_instruction(
        &Instruction::LoadT {
            dst: Reg(0),
            type_ref: ty,
        },
        &mut bytes,
    );
    assert!(decode_instruction(&bytes, &mut 0).is_err());
}

#[test]
fn mutable_dynamic_borrow_dispatches_through_its_actual_implementation() {
    let module = compile(
        "type Policy is protocol {fn advance(&mut self); fn read(&self)->Int;};
        type Counter is {value:Int};
        implement Policy for Counter {
            fn advance(&mut self) {self.value += 1;}
            fn read(&self)->Int {self.value}
        }
        fn inspect(value:&mut dyn Policy)->Int {value.advance(); value.read()}
        fn main()->Int {let mut value=Counter{value:72}; inspect(&mut value)}",
    );
    let decoded = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, decoded] {
        validate_module(&module).unwrap();
        let entry = module.find_function_by_name("main").unwrap();
        let mut vm = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module));
        vm.run_global_ctors().unwrap();
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    }
}

#[test]
fn dynamic_wire_requires_its_declared_format_revision() {
    let module = compile(
        "type Policy is protocol {fn read(&self)->Int;};
        fn inspect(value:&dyn Policy)->Int {73}",
    );
    let mut wire = serialize_module(&module).unwrap();
    wire[6..8].copy_from_slice(&54u16.to_le_bytes());
    assert!(deserialize_module(&wire).is_err());
}

#[test]
fn dynamic_applications_keep_outer_type_and_value_binders_distinct() {
    let module = compile(
        "type Policy<T, N: meta Int> is protocol {fn read(&self)->Int;};
        fn inspect<U, M: meta Int>(value:&dyn Policy<U,M>)->Int {73}",
    );
    validate_module(&module).unwrap();
    let function = module
        .functions
        .iter()
        .find(|function| module.get_string(function.name) == Some("inspect"))
        .unwrap();
    let declared = function.semantic_parameter_types().unwrap()[0]
        .as_ref()
        .unwrap();
    let TypeRef::Reference { inner, .. } = declared else {
        panic!("{declared:?}")
    };
    let TypeRef::Dynamic(object) = inner.as_ref() else {
        panic!("{inner:?}")
    };
    assert_eq!(object.bounds[0].args, [parameter(1), parameter(2)]);
    let mut substitution = verum_vbc::mono::TypeSubstitution::empty();
    substitution.bind(
        verum_vbc::types::TypeParamId(0),
        TypeRef::Concrete(TypeId::INT),
    );
    substitution.bind(verum_vbc::types::TypeParamId(1), TypeRef::ConstValue(17));
    let resolved = substitution.apply(inner).unwrap();
    let TypeRef::Dynamic(object) = resolved else {
        panic!("dynamic")
    };
    assert_eq!(
        object.bounds[0].args,
        [TypeRef::Concrete(TypeId::INT), TypeRef::ConstValue(17)]
    );
    for source in [
        "type Policy<N: meta Int> is protocol {fn read(&self)->Int;}; fn wrong<U>(value:&dyn Policy<U>)->Int {73}",
        "type Policy<T> is protocol {fn read(&self)->Int;}; fn wrong<N: meta Int>(value:&dyn Policy<N>)->Int {73}",
    ] {
        let ast = Parser::new(source).parse_module().unwrap();
        assert!(
            VbcCodegen::new().compile_module(&ast).is_err(),
            "kind confusion admitted: {source}"
        );
    }
}

#[test]
fn associated_dynamic_result_uses_the_exact_implementation_after_wire() {
    let module = compile(
        "type Policy is protocol {type Item; fn read(&self)->Self.Item;};
        type Counter is {value:Int};
        implement Policy for Counter {type Item=Int; fn read(&self)->Int {self.value}}
        fn inspect(value:&dyn Policy<Item=Int>)->Int {value.read()}
        fn main()->Int {let value=Counter{value:73}; inspect(&value)}",
    );
    let decoded = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    validate_module(&decoded).unwrap();
    let entry = decoded.find_function_by_name("main").unwrap();
    let mut vm = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(decoded));
    vm.run_global_ctors().unwrap();
    assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
}

#[test]
fn existential_subject_cannot_be_forged_as_a_protocol_value_argument() {
    let module = compile(
        "type Policy<N: meta Int> is protocol {fn read(&self)->Int;};
        fn inspect(value:&dyn Policy<17>)->Int {73}",
    );
    let mut candidate = module.clone();
    let function = candidate.find_function_by_name("inspect").unwrap();
    let TypeRef::Reference { inner, .. } = &mut candidate
        .get_function_mut(function)
        .unwrap()
        .semantic_params
        .as_mut()
        .unwrap()[0]
        .as_mut()
        .unwrap()
    else {
        panic!("reference formal")
    };
    let TypeRef::Dynamic(object) = inner.as_mut() else {
        panic!("dynamic subject")
    };
    object.bounds[0].args[0] = parameter(0);
    let decoded = deserialize_module(&serialize_module(&candidate).unwrap()).unwrap();
    assert!(
        validate_module(&decoded).is_err(),
        "existential type occupied a value slot"
    );
}

#[test]
fn nested_constructor_calls_receive_the_selected_record_field_context() {
    for payload in ["Int", "dyn Policy"] {
        let module = compile(&format!(
            "type Policy is protocol {{fn read(&self)->Int;}};
            type Carrier<T> is {{value:Int}};
            implement<T> Carrier<T> {{fn new()->Self {{Carrier{{value:73}}}}}}
            type Holder<T> is {{value:T}};
            implement<T> Holder<T> {{fn new(value:T)->Self {{Holder{{value:value}}}}}}
            type Registry is {{item:Holder<Carrier<{payload}>>}};
            fn create()->Registry {{Registry{{item:Holder.new(Carrier.new())}}}}
            fn main()->Int {{let registry=create(); registry.item.value.value}}"
        ));
        let decoded = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        validate_module(&decoded).unwrap();
        let create = decoded
            .get_function(decoded.find_function_by_name("create").unwrap())
            .unwrap();
        let body = verum_vbc::bytecode::decode_instructions(
            &decoded.bytecode[create.bytecode_offset as usize
                ..(create.bytecode_offset + create.bytecode_length) as usize],
        )
        .unwrap();
        let carrier_witness = body
            .iter()
            .find_map(|instruction| match instruction {
                verum_vbc::Instruction::CallG {
                    func_id, type_args, ..
                } if decoded
                    .get_function(verum_vbc::FunctionId(*func_id))
                    .and_then(|function| decoded.get_string(function.name))
                    == Some("Carrier.new") =>
                {
                    Some(type_args)
                }
                _ => None,
            })
            .expect("the nested nullary call carries the field's declaration");
        match carrier_witness.as_slice() {
            [TypeRef::Dynamic(subject)] if payload == "dyn Policy" => {
                assert_eq!(subject.bounds.len(), 1);
                let policy = decoded
                    .types
                    .iter()
                    .find(|protocol| decoded.get_string(protocol.name) == Some("Policy"))
                    .unwrap();
                assert_eq!(subject.bounds[0].protocol.0, policy.id.0);
            }
            [TypeRef::Concrete(TypeId::INT)] if payload == "Int" => {}
            other => panic!("wrong nested witness for {payload}: {other:?}"),
        }
        let entry = decoded.find_function_by_name("main").unwrap();
        let mut vm = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(decoded));
        vm.run_global_ctors().unwrap();
        let result = vm
            .execute_function(entry)
            .unwrap_or_else(|error| panic!("{payload}: {error:?}"));
        assert_eq!(result.try_as_i64(), Some(73));
    }
}
