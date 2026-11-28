//! Instance dispatch maps the caller's witness through the implementation's self type.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{codegen::VbcCodegen, interpreter::Interpreter};

#[test]
fn dispatch_distinguishes_two_instantiations_of_the_same_implementor() {
    let parsed = Parser::new(include_str!("fixtures/bound_receiver.vr"))
        .parse_module()
        .unwrap();
    let module = VbcCodegen::new().compile_module(&parsed).unwrap();
    let id = module.find_function_by_name("probe").unwrap();
    assert_eq!(
        Interpreter::new(Arc::new(module))
            .execute_function(id)
            .unwrap()
            .as_i64(),
        108
    );
}

#[test]
fn implementation_binders_follow_the_self_pattern_including_repeated_positions() {
    use verum_vbc::types::{TypeId, TypeParamId, TypeRef};
    let parsed = Parser::new(
        r#"
        type Pair<A, B> is { left: A, right: B };
        implement<B, A> Pair<A, B> { fn sample(&self) -> Int { A.size } }
    "#,
    )
    .parse_module()
    .unwrap();
    let module = VbcCodegen::new().compile_module(&parsed).unwrap();
    let f = module
        .get_function(module.find_function_by_name("Pair.sample").unwrap())
        .unwrap();
    let TypeRef::Reference { inner, .. } =
        f.semantic_parameter_types().unwrap()[0].as_ref().unwrap()
    else {
        panic!("missing self")
    };
    let TypeRef::Instantiated {
        base,
        args: pattern,
    } = inner.as_ref()
    else {
        panic!("missing generic owner")
    };
    let actual = TypeRef::Instantiated {
        base: *base,
        args: vec![
            TypeRef::Concrete(TypeId::BYTE),
            TypeRef::Concrete(TypeId::INT),
        ],
    };
    let args = f.receiver_type_arguments(&actual).unwrap();
    for (i, expected) in [TypeId::BYTE, TypeId::INT].into_iter().enumerate() {
        let TypeRef::Generic(id) = pattern[i] else {
            panic!("missing binder")
        };
        assert_eq!(
            f.generic_argument(&args, id),
            Some(&TypeRef::Concrete(expected))
        );
    }
    let mut repeated = f.clone();
    let TypeRef::Reference { inner, .. } = repeated.semantic_params.as_mut().unwrap()[0]
        .as_mut()
        .unwrap()
    else {
        unreachable!()
    };
    *inner = Box::new(TypeRef::Instantiated {
        base: *base,
        args: vec![TypeRef::Generic(TypeParamId(0)); 2],
    });
    assert_eq!(repeated.receiver_type_arguments(&actual), None);
    repeated.semantic_params = None;
    assert_eq!(repeated.receiver_type_arguments(&actual), None);
}

#[test]
fn a_receiver_declaration_cannot_escape_to_a_later_call() {
    use verum_vbc::{
        bytecode::decode_instruction,
        instruction::{Instruction, Opcode},
    };
    let parsed = Parser::new(include_str!("fixtures/bound_receiver.vr"))
        .parse_module()
        .unwrap();
    let mut module = VbcCodegen::new().compile_module(&parsed).unwrap();
    let id = module.find_function_by_name("measure").unwrap();
    let entry = module.find_function_by_name("probe").unwrap();
    let mut pc = module.get_function(id).unwrap().bytecode_offset as usize;
    assert!(matches!(
        decode_instruction(&module.bytecode, &mut pc).unwrap(),
        Instruction::BindMethodReceiver { .. }
    ));
    assert_eq!(module.bytecode[pc], Opcode::CallM.to_byte());
    module.bytecode[pc] = Opcode::RetV.to_byte();
    let error = verum_vbc::validate::validate_module(&module).unwrap_err();
    assert!(
        format!("{error:?}")
            .contains("receiver declaration must immediately precede a method call")
    );
    let error = Interpreter::new(Arc::new(module))
        .execute_function(entry)
        .unwrap_err();
    assert!(
        format!("{error:?}")
            .contains("receiver declaration must immediately precede a method call")
    );
}

#[test]
fn a_branch_cannot_skip_the_bound_receivers_declaration() {
    use verum_vbc::{
        Instruction,
        bytecode::{decode_instruction, encode_instruction},
    };

    let parsed = Parser::new(include_str!("fixtures/bound_receiver.vr"))
        .parse_module()
        .unwrap();
    let mut module = VbcCodegen::new().compile_module(&parsed).unwrap();
    let id = module.find_function_by_name("measure").unwrap();
    let descriptor = module.get_function(id).unwrap();
    let start = descriptor.bytecode_offset as usize;
    let original = module.bytecode[start..start + descriptor.bytecode_length as usize].to_vec();
    let mut prefix_len = 0;
    assert!(matches!(
        decode_instruction(&original, &mut prefix_len).unwrap(),
        Instruction::BindMethodReceiver { .. }
    ));
    let offset = module.bytecode.len() as u32;
    encode_instruction(
        &Instruction::Jmp {
            offset: prefix_len as i32,
        },
        &mut module.bytecode,
    );
    module.bytecode.extend_from_slice(&original);
    let len = module.bytecode.len() as u32 - offset;
    let descriptor = module.functions.iter_mut().find(|f| f.id == id).unwrap();
    descriptor.bytecode_offset = offset;
    descriptor.bytecode_length = len;
    let error = verum_vbc::validate::validate_module(&module).unwrap_err();
    assert!(format!("{error:?}").contains("JumpOutOfBounds"));
}

#[test]
fn a_bound_instance_method_uses_its_protocol_slot_with_a_same_named_sibling() {
    use verum_vbc::types::{TypeId, TypeRef};
    let ast = Parser::new(r#"
        module app;
        type First is protocol { fn marker(&self) -> Int; };
        type Second is protocol { fn marker(&self) -> Int; };
        implement First for Text { fn marker(&self) -> Int { 11 } }
        implement Second for Text { fn marker(&self) -> Int { 73 } }
        fn choose<T:Second>(input:&T) -> Int { input.marker() }
        fn probe() -> Int { choose(&"payload") }
    "#).parse_module().unwrap();
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    let mut module = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
    module.resolve_protocol_dispatch();
    let selector = module.strings.lookup("dyn:app.Second.marker").unwrap();
    let (target, witnesses) = module.try_bound_method(&TypeRef::Concrete(TypeId::TEXT), selector, 0)
        .unwrap().expect("declared Second.marker implementation");
    assert!(witnesses.is_empty());
    assert!(module.try_bound_method(&TypeRef::Concrete(TypeId::BOOL), selector, 0).is_err());
    let probe = module.find_function_by_name("app.probe").unwrap();
    let mut vm = Interpreter::new(Arc::new(module));
    assert_ne!(probe, target);
    assert_eq!(vm.execute_function(probe).unwrap().as_i64(), 73);
}

#[test]
fn a_local_generic_function_and_its_enclosing_method_each_use_their_own_slots() {
    let ast = Parser::new(r#"
        module lexical;
        type Mark is protocol { fn mark(&self) -> Int; };
        implement Mark for Text { fn mark(&self) -> Int { 7 } }
        implement Mark for Bool { fn mark(&self) -> Int { 3 } }
        type Holder<T> is { value:T };
        implement<T:Mark> Holder<T> {
            fn pair(&self, text:&Text) -> Int {
                fn score<U:Mark>(value:&U) -> Int { value.mark() }
                if T.id == Bool.id { score(text) * 10 + score(&self.value) } else { 0 }
            }
        }
        fn probe() -> Int { let item: Holder<Bool> = Holder { value:true }; item.pair(&"x") }
    "#).parse_module().unwrap();
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    let module = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
    verum_vbc::validate::validate_module(&module).unwrap();
    let entry = module.find_function_by_name("lexical.probe").unwrap();
    let mut vm = Interpreter::new(Arc::new(module));
    assert_eq!(vm.execute_function(entry).unwrap().as_i64(), 73);
}


#[test]
fn applied_protocols_bind_the_selected_implementations_own_arguments() {
    let source = concat!(
        "module application;\n",
        include_str!("fixtures/applied_receiver_witness.vr")
    );
    let ast = Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    let restored = verum_vbc::deserialize::deserialize_module_validated(&wire).unwrap();
    for module in [module, restored] {
        use verum_vbc::types::{TypeId, TypeRef};
        let named = |name| {
            module
                .types
                .iter()
                .find(|t| module.get_string(t.name) == Some(name))
                .unwrap()
                .id
        };
        let owner = named("Reader");
        let mark = named("Mark");
        let read = named("Read");
        let receiver = TypeRef::Concrete(owner);
        let selector = module.strings.lookup("dyn:application.Mark.mark").unwrap();
        let application =
            |protocol, value| TypeRef::instantiated(protocol, [TypeRef::Concrete(value)]);
        let integer = application(mark, TypeId::INT);
        let boolean = application(mark, TypeId::BOOL);
        let select = |image: &verum_vbc::module::VbcModule, witness: &[TypeRef]| {
            image.try_applied_bound_method(&receiver, selector, 0, witness.first(), &[])
        };
        let integer_body = select(&module, std::slice::from_ref(&integer))
            .unwrap()
            .unwrap()
            .0;
        let boolean_body = select(&module, std::slice::from_ref(&boolean))
            .unwrap()
            .unwrap()
            .0;
        assert_ne!(integer_body, boolean_body);
        assert!(select(&module, &[]).is_err());
        assert!(select(&module, &[application(read, TypeId::INT)]).is_err());
        assert!(select(&module, &[application(mark, TypeId::TEXT)]).is_err());

        let relay = module
            .get_function(
                module
                    .find_function_by_name("application.read_through")
                    .unwrap(),
            )
            .unwrap();
        let start = relay.bytecode_offset as usize;
        let instructions = verum_vbc::bytecode::decode_instructions(
            &module.bytecode[start..start + relay.bytecode_length as usize],
        )
        .unwrap();
        let paired = instructions
            .windows(2)
            .find_map(|pair| match pair {
                [
                    verum_vbc::Instruction::BindProtocolReceiver {
                        application: TypeRef::Instantiated { base, .. },
                        ..
                    },
                    call @ verum_vbc::Instruction::CallM { .. },
                ] if *base == read => Some(call.clone()),
                _ => None,
            })
            .expect("receiver and protocol application share one prefix for the selected call");
        let mut graph = verum_vbc::mono::InstantiationGraph::new();
        let read_application = application(read, TypeId::INT);
        let read_selector = module.strings.lookup("dyn:application.Read.read").unwrap();
        let (read_body, read_arguments) = module
            .try_applied_bound_method(&receiver, read_selector, 1, Some(&read_application), &[])
            .unwrap()
            .unwrap();
        let prefix = verum_vbc::Instruction::BindProtocolReceiver {
            receiver: receiver.clone(),
            application: read_application,
        };
        verum_vbc::mono::discover_call_instantiations(&module, &[prefix, paired], 0, &mut graph)
            .unwrap();
        assert!(graph.contains(read_body, &read_arguments));

        let mut forged = module.clone();
        let row = forged
            .types
            .iter_mut()
            .find(|ty| ty.id == owner)
            .unwrap()
            .protocols
            .iter_mut()
            .find(|row| row.methods.first() == Some(&integer_body.0))
            .unwrap();
        row.methods[0] = boolean_body.0;
        assert!(select(&forged, std::slice::from_ref(&integer)).is_err());

        let mut ambiguous = module.clone();
        let row = ambiguous
            .types
            .iter_mut()
            .find(|ty| ty.id == owner)
            .unwrap()
            .protocols
            .iter_mut()
            .find(|row| row.methods.first() == Some(&boolean_body.0))
            .unwrap();
        row.application.as_mut().unwrap().arguments = vec![TypeRef::Concrete(TypeId::INT)];
        assert!(
            select(&ambiguous, std::slice::from_ref(&integer))
                .unwrap_err()
                .contains("multiple implementations")
        );
        let entry = module.find_function_by_name("application.probe").unwrap();
        let mut vm = Interpreter::try_new_validated(Arc::new(module)).unwrap();
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    }
}

#[test]
fn applied_implementation_collection_precedes_its_foreign_protocol_descriptor() {
    use verum_vbc::codegen::{CodegenConfig, ItemFailurePolicy};
    let reader = Parser::new(
        r#"
        module app.reader;
        mount app.protocol.Read;
        public type Reader is {};
        implement<X> Read<X> for Reader {
            fn value(&self) -> Int { if X.id == Bool.id { 73 } else { 0 } }
        }
    "#,
    )
    .parse_module()
    .unwrap();
    let protocol = Parser::new(
        r#"
        module app.protocol;
        public type Read<X> is protocol { fn value(&self) -> Int; };
    "#,
    )
    .parse_module()
    .unwrap();
    let client = Parser::new(
        r#"
        module app.client;
        mount app.protocol.Read;
        mount app.reader.Reader;
        fn relay<X, R: Read<X>>(r: &R, x: X) -> Int { r.value() }
        fn probe() -> Int { let r = Reader {}; relay(&r, true) }
    "#,
    )
    .parse_module()
    .unwrap();
    for files in [[&reader, &client, &protocol], [&protocol, &client, &reader]] {
        let mut cg = VbcCodegen::with_config(CodegenConfig::new("app"));
        cg.collect_unit_declarations(&files).unwrap();
        cg.resolve_pending_imports();
        cg.compile_unit_items(&files, ItemFailurePolicy::Strict)
            .unwrap();
        let original = cg.finalize_module_from_state().unwrap();
        let wire = verum_vbc::serialize::serialize_module(&original).unwrap();
        for image in [
            original,
            verum_vbc::deserialize::deserialize_module_validated(&wire).unwrap(),
        ] {
            let probe = image.find_function_by_name("app.client.probe").unwrap();
            let mut vm = Interpreter::try_new_validated(Arc::new(image)).unwrap();
            assert_eq!(vm.execute_function(probe).unwrap().try_as_i64(), Some(73));
        }
    }
}

#[test]
fn a_dynamic_witness_selects_the_actual_objects_exact_protocol_application() {
    use verum_vbc::types::{DynamicProtocolType, ProtocolApplication, ProtocolId, TypeId, TypeRef};
    let source = concat!(
        "module application;\n",
        include_str!("fixtures/dynamic_receivers_applied_method.vr")
    );
    let ast = Parser::new(source).parse_module().unwrap();
    let original = VbcCodegen::new().compile_module(&ast).unwrap();
    let serialized = verum_vbc::serialize::serialize_module(&original).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module_validated(&serialized).unwrap();
    for module in [original, wire] {
        let protocol = module
            .types
            .iter()
            .find(|ty| module.get_string(ty.name) == Some("Mark"))
            .unwrap()
            .id;
        let object = DynamicProtocolType {
            bounds: [ProtocolApplication {
                protocol: ProtocolId(protocol.0),
                args: vec![TypeRef::Concrete(TypeId::INT)],
            }]
            .into_iter()
            .collect(),
            bindings: Default::default(),
        };
        let selected = module.strings.lookup("dyn:application.Mark.apply").unwrap();
        let sibling = module
            .strings
            .lookup("dyn:application.Other.apply")
            .unwrap();
        assert!(
            module
                .object_member_application(&object, sibling, None)
                .is_err()
        );
        let wrong = TypeRef::instantiated(protocol, [TypeRef::Concrete(TypeId::BOOL)]);
        assert!(
            module
                .object_method_candidates(&object, selected, 1, Some(&wrong), &[])
                .is_err()
        );
        let entries = module
            .object_method_candidates(&object, selected, 1, None, &[])
            .unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|(_, _, arguments)| arguments.is_empty()));
        assert!(entries
            .iter()
            .any(|(_, _, arguments)| arguments.as_slice() == [TypeRef::Concrete(TypeId::INT)]));
        let entry = module.find_function_by_name("application.main").unwrap();
        let erased = module.find_function_by_name("application.integer").unwrap();
        let mut vm = Interpreter::try_new_validated(Arc::new(module)).unwrap();
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
        let missing = vm.execute_function_with_args(erased, &[verum_vbc::value::Value::from_i64(9)])
            .expect_err("the actual primitive owner has no Mark<Int> implementation");
        assert!(matches!(missing, verum_vbc::interpreter::InterpreterError::InvalidBytecode { .. }), "{missing:?}");
    }
}
