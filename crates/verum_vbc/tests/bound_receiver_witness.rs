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
