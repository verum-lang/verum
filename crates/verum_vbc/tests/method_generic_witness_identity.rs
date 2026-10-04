#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::bytecode::decode_instruction;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::instruction::Instruction;
use verum_vbc::types::TypeRef;

#[test]
fn explicit_method_generic_argument_reaches_its_declared_slot() {
    let source = r#"
type Answer is { value: Int };
implement Answer { fn from_value(value: Int) -> Answer { Answer { value } } }
type Factory<Item> is { value: Item };
implement<Item> Factory<Item> {
    fn build<Output>(self) -> Output { Output.from_value(self.value) }
}
fn probe() -> Int {
    let factory: Factory<Int> = Factory { value: 7 };
    let result = factory.build<Answer>();
    result.value
}
"#;
    let ast = Parser::new(source).parse_module().expect("parse");
    let module = VbcCodegen::with_config(CodegenConfig::new("method_witness_pins"))
        .compile_module(&ast)
        .expect("compile");
    let answer = module
        .types
        .iter()
        .find(|t| {
            module
                .get_string(t.name)
                .is_some_and(|n| n == "Answer" || n.ends_with(".Answer"))
        })
        .expect("Answer")
        .id;
    let mut calls = Vec::new();
    for function in &module.functions {
        let mut pc = function.bytecode_offset as usize;
        let end = pc + function.bytecode_length as usize;
        while pc < end {
            match decode_instruction(&module.bytecode, &mut pc).expect("instruction") {
                Instruction::CallG {
                    func_id, type_args, ..
                } => {
                    let name = module
                        .get_string(module.functions[func_id as usize].name)
                        .unwrap();
                    if name.ends_with("Factory.build") {
                        calls.push(type_args);
                    }
                }
                _ => {}
            }
        }
    }
    assert_eq!(calls.len(), 1, "generic call missing: {calls:?}");
    assert_eq!(
        calls[0].get(1),
        Some(&TypeRef::Concrete(answer)),
        "explicit Output is slot 1 after impl Item: {calls:?}"
    );
    let entry = module
        .functions
        .iter()
        .find(|f| module.get_string(f.name).unwrap().ends_with("probe"))
        .unwrap()
        .id;
    let value = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
        .execute_function(entry)
        .expect("execute explicit method type");
    assert_eq!(value.as_i64(), 7);
}

fn compile(source: &str) -> verum_vbc::module::VbcModule {
    let ast = Parser::new(source).parse_module().expect("parse");
    VbcCodegen::with_config(CodegenConfig::new("method_witness_pins"))
        .compile_module(&ast)
        .expect("compile")
}

fn call(module: &verum_vbc::module::VbcModule, suffix: &str) -> Vec<TypeRef> {
    for f in &module.functions {
        let mut pc = f.bytecode_offset as usize;
        let end = pc + f.bytecode_length as usize;
        while pc < end {
            if let Instruction::CallG {
                func_id, type_args, ..
            } = decode_instruction(&module.bytecode, &mut pc).unwrap()
            {
                if module
                    .get_string(module.functions[func_id as usize].name)
                    .unwrap()
                    .ends_with(suffix)
                {
                    return type_args;
                }
            }
        }
    }
    panic!("missing CallG {suffix}");
}

fn type_id(module: &verum_vbc::module::VbcModule, name: &str) -> verum_vbc::types::TypeId {
    module
        .types
        .iter()
        .find(|t| {
            module
                .get_string(t.name)
                .is_some_and(|n| n == name || n.ends_with(&format!(".{name}")))
        })
        .unwrap()
        .id
}

#[test]
fn partial_explicit_arguments_leave_later_method_parameters_inferred() {
    let module = compile(
        r#"
type Answer is { value: Int };
implement Answer { fn from_value(value: Int) -> Answer { Answer { value } } }
type Factory<Item> is { value: Item };
implement<Item> Factory<Item> {
    fn build<Output, Other>(self, other: Other) -> Output { Output.from_value(self.value) }
}
fn probe() -> Int {
    let factory: Factory<Int> = Factory { value: 7 };
    factory.build<Answer>("unused").value
}
"#,
    );
    let args = call(&module, "Factory.build");
    assert_eq!(
        args,
        vec![
            TypeRef::Concrete(verum_vbc::types::TypeId::INT),
            TypeRef::Concrete(type_id(&module, "Answer")),
            TypeRef::Concrete(verum_vbc::types::TypeId::TEXT)
        ]
    );
}

#[test]
fn nested_explicit_hole_uses_full_let_annotation() {
    let module = compile(
        r#"
type Factory<Item> is { value: Item };
implement<Item> Factory<Item> { fn build<Output>(self) -> Output { Output.default() } }
fn probe() -> Int {
    let factory: Factory<Int> = Factory { value: 7 };
    let result: List<Maybe<Int>> = { factory.build<List<_>>() };
    7
}
"#,
    );
    let args = call(&module, "Factory.build");
    let TypeRef::Instantiated { base, args: inner } = &args[1] else {
        panic!("full result lost: {args:?}")
    };
    assert_eq!(*base, verum_vbc::types::TypeId::LIST);
    let TypeRef::Instantiated { args: nested, .. } = &inner[0] else {
        panic!("nested result lost: {args:?}")
    };
    assert_eq!(nested, &[TypeRef::Concrete(verum_vbc::types::TypeId::INT)]);
}

#[test]
fn unresolved_nested_hole_never_inherits_unrelated_outer_return() {
    let module = compile(
        r#"
type Factory<Item> is { value: Item };
implement<Item> Factory<Item> { fn build<Output>(self) -> Output { Output.default() } }
fn probe() -> Int {
    let factory: Factory<Int> = Factory { value: 7 };
    let result = factory.build<List<_>>();
    7
}
"#,
    );
    assert_eq!(
        call(&module, "Factory.build")[1],
        TypeRef::Generic(verum_vbc::types::TypeParamId(1))
    );
}

#[test]
fn method_shadow_slot_does_not_overwrite_parent_parameter() {
    let module = compile(
        r#"
type Answer is { value: Int };
implement Answer { fn from_value(value: Int) -> Answer { Answer { value } } }
type Factory<Item> is { value: Item };
implement<Item> Factory<Item> { fn build<Item>(self) -> Item { Item.from_value(7) } }
fn probe() -> Int {
    let factory: Factory<Int> = Factory { value: 7 };
    factory.build<Answer>().value
}
"#,
    );
    let descriptor = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .unwrap()
                .ends_with("Factory.build")
        })
        .unwrap();
    assert_eq!(
        descriptor.explicit_type_param_ids,
        vec![Some(verum_vbc::types::TypeParamId(0x8000))]
    );
    let args = call(&module, "Factory.build");
    assert_eq!(args[0], TypeRef::Concrete(verum_vbc::types::TypeId::INT));
    assert_eq!(args.len(), 2, "shadow witness must stay compact");
    assert_eq!(args[1], TypeRef::Concrete(type_id(&module, "Answer")));
}

#[test]
fn forward_declared_method_carries_explicit_slot_before_body_compilation() {
    let module = compile(
        r#"
type Answer is { value: Int };
type Factory<Item> is { value: Item };
fn probe() -> Int {
    let factory: Factory<Int> = Factory { value: 7 };
    factory.build<Answer>().value
}
implement Answer { fn from_value(value: Int) -> Answer { Answer { value } } }
implement<Item> Factory<Item> { fn build<Output>(self) -> Output { Output.from_value(self.value) } }
"#,
    );
    assert_eq!(
        call(&module, "Factory.build")[1],
        TypeRef::Concrete(type_id(&module, "Answer"))
    );
}

#[test]
fn explicit_caller_generic_remains_a_caller_witness() {
    let module = compile(
        r#"
type Factory<Item> is { value: Item };
implement<Item> Factory<Item> { fn build<Output>(self) -> Output { Output.default() } }
fn probe<Caller>(factory: Factory<Int>) -> Caller { factory.build<Caller>() }
"#,
    );
    assert_eq!(
        call(&module, "Factory.build")[1],
        TypeRef::Generic(verum_vbc::types::TypeParamId(0))
    );
}

#[test]
fn declaration_slots_survive_wire_and_archive_member_roundtrip() {
    use verum_vbc::types::TypeParamId;
    let mut module = compile("fn probe() -> Int { 7 }");
    module.functions[0].explicit_type_param_ids =
        vec![Some(TypeParamId(3)), None, Some(TypeParamId(0x8000))];
    for encoded in [
        verum_vbc::serialize::serialize_module(&module).unwrap(),
        verum_vbc::serialize::serialize_archive_member(&module).unwrap(),
    ] {
        let decoded = verum_vbc::deserialize::deserialize_module(&encoded).unwrap();
        assert_eq!(
            decoded.functions[0].explicit_type_param_ids,
            module.functions[0].explicit_type_param_ids
        );
    }
}

#[test]
fn caller_method_shadow_uses_its_signature_id() {
    let module = compile(
        r#"
fn helper<Result>() -> Result { Result.default() }
type Factory<Item> is { value: Item };
implement<Item> Factory<Item> {
    fn build<Item>(self) -> Item { helper<Item>() }
}
"#,
    );
    assert_eq!(
        call(&module, "helper")[0],
        TypeRef::Generic(verum_vbc::types::TypeParamId(0x8000))
    );
}

#[test]
fn implicit_method_parameter_is_not_an_explicit_slot() {
    let module = compile(
        r#"
type Answer is { value: Int };
implement Answer { fn from_value(value: Int) -> Answer { Answer { value } } }
type Factory<Item> is { value: Item };
implement<Item> Factory<Item> {
    fn build<{Hidden}, Output>(self, hidden: Hidden) -> Output { Output.from_value(7) }
}
fn probe() -> Int {
    let factory: Factory<Int> = Factory { value: 7 };
    factory.build<Answer>("unused").value
}
"#,
    );
    let args = call(&module, "Factory.build");
    assert_eq!(args[1], TypeRef::Concrete(verum_vbc::types::TypeId::TEXT));
    assert_eq!(args[2], TypeRef::Concrete(type_id(&module, "Answer")));
}

#[test]
fn non_type_generic_slots_do_not_shift_explicit_type_arguments() {
    let ast = Parser::new("fn sample<const Count: Int, Output>() -> Output { Output.default() }")
        .parse_module()
        .unwrap();
    let verum_ast::ItemKind::Function(func) = &ast.items[0].kind else {
        panic!()
    };
    let slots = VbcCodegen::declared_explicit_type_param_ids(func, &[], &[]);
    assert_eq!(slots, vec![None, Some(verum_vbc::types::TypeParamId(0))]);
    let module = compile("fn sample<const Count: Int, Output>() -> Output { Output.default() }");
    assert_eq!(
        module
            .functions
            .iter()
            .find(|f| module.get_string(f.name).unwrap().ends_with("sample"))
            .unwrap()
            .explicit_type_param_ids,
        slots
    );
}

#[test]
fn context_parameter_does_not_renumber_registered_type_slot() {
    let source = "fn sample<using C, Output>() -> Output { Output.default() }";
    let ast = Parser::new(source).parse_module().unwrap();
    let verum_ast::ItemKind::Function(func) = &ast.items[0].kind else {
        panic!()
    };
    let slots = VbcCodegen::declared_explicit_type_param_ids(func, &[], &[]);
    assert_eq!(slots, vec![None, Some(verum_vbc::types::TypeParamId(0))]);
    let module = compile(source);
    assert_eq!(
        module
            .functions
            .iter()
            .find(|f| module.get_string(f.name).unwrap().ends_with("sample"))
            .unwrap()
            .explicit_type_param_ids,
        slots
    );
}

#[test]
fn bootstrap_declaration_authority_matches_body_descriptor() {
    let source = r#"
type Factory<Zed, First> is { zed: Zed, first: First };
implement<Zed, First> Factory<Zed, First> {
    fn build<First, Output>(self) -> Output { Output.default() }
}
"#;
    let ast = Parser::new(source).parse_module().unwrap();
    let verum_ast::ItemKind::Impl(implementation) = &ast.items[1].kind else {
        panic!()
    };
    let verum_ast::decl::ImplItemKind::Function(func) = &implementation.items[0].kind else {
        panic!()
    };
    let slots = VbcCodegen::declared_explicit_type_param_ids(
        func,
        &["Zed".into(), "First".into()],
        &["Zed".into(), "First".into()],
    );
    assert_eq!(
        slots,
        vec![
            Some(verum_vbc::types::TypeParamId(0x8000)),
            Some(verum_vbc::types::TypeParamId(2))
        ]
    );
    let module = compile(source);
    let method = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .unwrap()
                .ends_with("Factory.build")
        })
        .unwrap();
    assert_eq!(method.explicit_type_param_ids, slots);
}

#[test]
fn static_method_keeps_receiver_and_explicit_method_arguments() {
    let module = compile(
        r#"
type Answer is { value: Int };
implement Answer { fn from_value(value: Int) -> Answer { Answer { value } } }
type Factory<Item> is { value: Item };
implement<Item> Factory<Item> { fn build<Output>() -> Output { Output.from_value(7) } }
fn probe() -> Int { Factory<Int>.build<Answer>().value }
"#,
    );
    let args = call(&module, "Factory.build");
    assert_eq!(
        args,
        vec![
            TypeRef::Concrete(verum_vbc::types::TypeId::INT),
            TypeRef::Concrete(type_id(&module, "Answer"))
        ]
    );
}

#[test]
fn self_describing_legacy_descriptor_defaults_the_new_carrier() {
    let descriptor = verum_vbc::module::FunctionDescriptor::default();
    let mut value = serde_json::to_value(descriptor).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .remove("explicit_type_param_ids");
    let decoded: verum_vbc::module::FunctionDescriptor = serde_json::from_value(value).unwrap();
    assert!(decoded.explicit_type_param_ids.is_empty());
}

#[test]
fn forward_method_nested_hole_uses_registered_generic_return() {
    let module = compile(
        r#"
type Factory<Item> is { value: Item };
fn probe() -> Int {
    let factory: Factory<Int> = Factory { value: 7 };
    let result: List<Maybe<Int>> = factory.build<List<_>>();
    7
}
implement<Item> Factory<Item> { fn build<Output>(self) -> Output { Output.default() } }
"#,
    );
    let args = call(&module, "Factory.build");
    assert!(
        matches!(&args[1], TypeRef::Instantiated { base, args } if *base == verum_vbc::types::TypeId::LIST && matches!(&args[0], TypeRef::Instantiated { .. })),
        "{args:?}"
    );
}

#[test]
fn mixed_static_receiver_keeps_its_value_witness() {
    let module = compile(
        r#"
type Sized<Item, const Count: Int> is { value: Item };
implement<Item, const Count: Int> Sized<Item, Count> {
    fn make(value: Item) -> Self { Self { value } }
}
fn probe() -> Int {
    let value: Sized<Int, 9> = Sized<Int, 9>.make(7);
    7
}
"#,
    );
    assert_eq!(
        call(&module, "Sized.make"),
        vec![
            TypeRef::Concrete(verum_vbc::types::TypeId::INT),
            TypeRef::ConstValue(9)
        ]
    );
}

#[test]
fn const_only_receiver_keeps_the_existing_value_witness() {
    let module = compile(
        r#"
type Fixed<const Count: Int> is { marker: Int };
implement<const Count: Int> Fixed<Count> { fn size(self) -> Int { Count } }
fn probe() -> Int {
    let value: Fixed<9> = Fixed { marker: 1 };
    value.size()
}
"#,
    );
    assert_eq!(call(&module, "Fixed.size"), vec![TypeRef::ConstValue(9)]);
    let entry = module
        .functions
        .iter()
        .find(|f| module.get_string(f.name).unwrap().ends_with("probe"))
        .unwrap()
        .id;
    let value = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
        .execute_function(entry)
        .unwrap();
    assert_eq!(value.as_i64(), 9);
}
