#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::module::{FunctionDescriptor, VbcModule};
use verum_vbc::types::{TypeId, TypeRef};

fn compile(source: &str) -> VbcModule {
    let ast = Parser::new(source).parse_module().expect("parse");
    VbcCodegen::with_config(CodegenConfig::new("callable"))
        .compile_module(&ast)
        .expect("compile")
}

fn nominal(module: &VbcModule, name: &str) -> TypeRef {
    let desc = module
        .types
        .iter()
        .find(|t| module.get_string(t.name) == Some(name))
        .expect(name);
    TypeRef::Concrete(desc.id)
}

fn closures(module: &VbcModule) -> verum_common::List<&FunctionDescriptor> {
    module
        .functions
        .iter()
        .filter(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n.contains("$closure$"))
        })
        .collect()
}

#[test]
fn annotated_nominal_parameter_reaches_closure_descriptor() {
    let module = compile(
        "type Payload is { value: Int }; fn probe() -> Int { let f = |x: Payload| -> Payload { x }; 0 }",
    );
    let f = closures(&module)[0];
    assert_eq!(f.params[1].type_ref, nominal(&module, "Payload"));
    assert_eq!(f.return_type, nominal(&module, "Payload"));
}

#[test]
fn closure_result_is_inferred_in_its_parameter_scope() {
    let module =
        compile("type Payload is { value: Int }; fn probe() -> Int { let f = |x: Payload| x; 0 }");
    assert_eq!(
        closures(&module)[0].return_type,
        nominal(&module, "Payload")
    );
}

#[test]
fn declared_callable_argument_types_contextualize_lambda_without_adapter_name() {
    let module = compile(
        r#"
type Payload is { value: Int };
type Stage<T> is { value: T };
implement<T> Stage<T> { fn transform<R, F: fn(T) -> R>(self, f: F) -> F { f } }
fn probe(stage: Stage<Payload>) -> Int { let f = stage.transform(|x| x); 0 }
"#,
    );
    let f = closures(&module)[0];
    assert_eq!(f.params[1].type_ref, nominal(&module, "Payload"));
    assert_eq!(f.return_type, nominal(&module, "Payload"));
}

#[test]
fn sibling_closures_keep_distinct_nominal_parameter_and_result_types() {
    let module = compile(
        r#"
type First is { value: Int };
type Second is { value: Int };
fn probe() -> Int { let f = |x: First| x; let g = |x: Second| x; 0 }
"#,
    );
    let fs = closures(&module);
    assert_eq!(fs.len(), 2);
    for (f, name) in fs.iter().zip(["First", "Second"]) {
        assert_eq!(f.params[1].type_ref, nominal(&module, name));
        assert_eq!(f.return_type, nominal(&module, name));
    }
}

#[test]
fn explicit_primitive_closure_signature_remains_exact() {
    let module = compile("fn probe() -> Int { let f = |x: Bool| -> Bool { x }; 0 }");
    let f = closures(&module)[0];
    assert_eq!(f.params[1].type_ref, TypeRef::Concrete(TypeId::BOOL));
    assert_eq!(f.return_type, TypeRef::Concrete(TypeId::BOOL));
}

#[test]
fn contextual_callback_slots_are_independent() {
    let module = compile(
        r#"
type Payload is { value: Int };
type Stage<T> is { value: T };
implement<T> Stage<T> { fn both(self, first: fn(T) -> T, second: fn(Bool) -> Bool) -> Int { 0 } }
fn probe(stage: Stage<Payload>) -> Int { stage.both(|x| x, |x| x) }
"#,
    );
    let fs = closures(&module);
    assert_eq!(fs.len(), 2);
    assert_eq!(fs[0].params[1].type_ref, nominal(&module, "Payload"));
    assert_eq!(fs[1].params[1].type_ref, TypeRef::Concrete(TypeId::BOOL));
    assert_eq!(fs[0].return_type, nominal(&module, "Payload"));
    assert_eq!(fs[1].return_type, TypeRef::Concrete(TypeId::BOOL));
}

#[test]
fn unrelated_same_named_method_cannot_supply_callback_context() {
    let module = compile(
        r#"
type Payload is { value: Int };
type Decoy is { other: Int };
type Stage<T> is { value: T };
type Other is { value: Int };
implement Other { fn transform(self, f: fn(Decoy) -> Decoy) -> Int { 0 } }
implement<T> Stage<T> { fn transform(self, f: fn(T) -> T) -> Int { 0 } }
fn probe(stage: Stage<Payload>) -> Int { stage.transform(|x| x) }
"#,
    );
    let f = closures(&module)[0];
    assert_eq!(f.params[1].type_ref, nominal(&module, "Payload"));
    assert_eq!(f.return_type, nominal(&module, "Payload"));
}

#[test]
fn nested_nominal_annotation_keeps_all_type_arguments() {
    let module = compile(
        r#"
type Payload is { value: Int };
type Envelope<T> is { value: T };
fn probe() -> Int { let f = |x: Envelope<Payload>| x; 0 }
"#,
    );
    let TypeRef::Concrete(base) = nominal(&module, "Envelope") else {
        panic!("nominal")
    };
    let expected = TypeRef::Instantiated {
        base,
        args: vec![nominal(&module, "Payload")],
    };
    let f = closures(&module)[0];
    assert_eq!(f.params[1].type_ref, expected);
    assert_eq!(f.return_type, expected);
}

#[test]
fn reference_annotation_is_preserved_in_parameter_and_result() {
    use verum_vbc::types::{CbgrTier, Mutability};
    let module =
        compile("type Payload is { value: Int }; fn probe() -> Int { let f = |x: &Payload| x; 0 }");
    let expected = TypeRef::Reference {
        inner: Box::new(nominal(&module, "Payload")),
        mutability: Mutability::Immutable,
        tier: CbgrTier::Tier0,
    };
    let f = closures(&module)[0];
    assert_eq!(f.params[1].type_ref, expected);
    assert_eq!(f.return_type, expected);
}

#[test]
fn caller_generic_annotation_stays_generic_despite_same_named_nominal() {
    use verum_vbc::types::TypeParamId;
    let module = compile("type T is { value: Int }; fn probe<T>() -> Int { let f = |x: T| x; 0 }");
    let f = closures(&module)[0];
    assert_eq!(f.params[1].type_ref, TypeRef::Generic(TypeParamId(0)));
    assert_eq!(f.return_type, TypeRef::Generic(TypeParamId(0)));
}

#[test]
fn tuple_owner_argument_is_one_contextual_parameter() {
    let module = compile(
        r#"
type Left is { value: Int };
type Right is { value: Int };
type Stage<T> is { value: T };
implement<T> Stage<T> { fn transform(self, f: fn(T) -> T) -> Int { 0 } }
fn probe(stage: Stage<(Left, Right)>) -> Int { stage.transform(|x| x) }
"#,
    );
    let expected = TypeRef::Tuple(vec![nominal(&module, "Left"), nominal(&module, "Right")]);
    let f = closures(&module)[0];
    assert_eq!(f.params[1].type_ref, expected);
    assert_eq!(f.return_type, expected);
}

#[test]
fn function_owner_argument_preserves_full_signature() {
    let module = compile(
        r#"
type Left is { value: Int };
type Right is { value: Int };
type Output is { value: Int };
type Stage<T> is { value: T };
implement<T> Stage<T> { fn transform(self, f: fn(T) -> T) -> Int { 0 } }
fn probe(stage: Stage<fn(Left, Right) -> Output>) -> Int { stage.transform(|x| x) }
"#,
    );
    let expected = TypeRef::Function {
        params: vec![nominal(&module, "Left"), nominal(&module, "Right")],
        return_type: Box::new(nominal(&module, "Output")),
        contexts: Default::default(),
    };
    let f = closures(&module)[0];
    assert_eq!(f.params[1].type_ref, expected);
    assert_eq!(f.return_type, expected);
}

#[test]
fn static_callable_parameters_do_not_skip_a_receiver_slot() {
    let module = compile(
        r#"
type Payload is { value: Int };
type Stage is { value: Int };
implement Stage { fn transform(first: fn(Payload) -> Payload, second: fn(Bool) -> Bool) -> Int { 0 } }
fn probe() -> Int { Stage.transform(|x| x, |x| x) }
"#,
    );
    let fs = closures(&module);
    assert_eq!(fs[0].params[1].type_ref, nominal(&module, "Payload"));
    assert_eq!(fs[1].params[1].type_ref, TypeRef::Concrete(TypeId::BOOL));
}

#[test]
fn equal_offsets_in_distinct_source_files_keep_callback_contexts_separate() {
    let source = r#"
type Payload is { value: Int };
type Stage<T> is { value: T };
implement<T> Stage<T> { fn both(self, first: fn(T) -> T, second: fn(Bool) -> Bool) -> Int { 0 } }
fn probe(stage: Stage<Payload>) -> Int { stage.both(|x| x, |x| x) }
"#;
    let mut ast = Parser::new(source).parse_module().expect("parse");
    for item in &mut ast.items {
        let verum_ast::ItemKind::Function(func) = &mut item.kind else {
            continue;
        };
        let Some(verum_ast::FunctionBody::Block(block)) = func.body.as_mut() else {
            continue;
        };
        let Some(expr) = block.expr.as_mut() else {
            continue;
        };
        let verum_ast::ExprKind::MethodCall { args, .. } = &mut expr.kind else {
            continue;
        };
        for (index, arg) in args.iter_mut().enumerate() {
            arg.span = verum_ast::Span::new(10, 20, verum_ast::FileId::new(index as u32 + 1));
        }
    }
    let module = VbcCodegen::with_config(CodegenConfig::new("callable"))
        .compile_module(&ast)
        .expect("compile");
    let fs = closures(&module);
    assert_eq!(fs[0].params[1].type_ref, nominal(&module, "Payload"));
    assert_eq!(fs[1].params[1].type_ref, TypeRef::Concrete(TypeId::BOOL));
}

fn method_calls(module: &VbcModule, suffix: &str) -> verum_common::List<Vec<TypeRef>> {
    let mut calls = verum_common::List::new();
    for function in &module.functions {
        let mut pc = function.bytecode_offset as usize;
        let end = pc + function.bytecode_length as usize;
        while pc < end {
            if let verum_vbc::instruction::Instruction::CallG {
                func_id, type_args, ..
            } = verum_vbc::bytecode::decode_instruction(&module.bytecode, &mut pc)
                .expect("instruction")
            {
                if module
                    .functions
                    .iter()
                    .find(|f| f.id.0 == func_id)
                    .and_then(|f| module.get_string(f.name))
                    .is_some_and(|name| name.ends_with(suffix))
                {
                    calls.push(type_args);
                }
            }
        }
    }
    calls
}

#[test]
fn proven_callable_signature_binds_the_declared_method_parameter() {
    let module = compile(
        r#"
type Payload is { value: Int };
type Stage<T> is { value: T };
implement<T> Stage<T> { fn transform<R, F: fn(T) -> R>(self, f: F) -> F { f } }
fn probe(stage: Stage<Payload>) -> Int { let f = stage.transform(|x| x); 0 }
"#,
    );
    let payload = nominal(&module, "Payload");
    let function = TypeRef::Function {
        params: vec![payload.clone()],
        return_type: Box::new(payload.clone()),
        contexts: Default::default(),
    };
    assert_eq!(
        method_calls(&module, "Stage.transform").as_slice(),
        &[vec![payload.clone(), payload, function]]
    );
}

#[test]
fn untyped_callable_abi_fallback_is_not_a_generic_binding() {
    let module = compile(
        r#"
type Stage<T> is { value: T };
implement<T> Stage<T> { fn transform<F>(self, f: F) -> F { f } }
fn probe(stage: Stage<Int>) -> Int { let f = stage.transform(|x| x); 0 }
"#,
    );
    let calls = method_calls(&module, "Stage.transform");
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0][1],
        TypeRef::Generic(verum_vbc::types::TypeParamId(1))
    );
}

#[test]
fn callback_value_executes_with_the_same_proven_nominal_signature() {
    let module = compile(
        r#"
type Payload is { value: Int };
type Stage<T> is { value: T };
implement<T> Stage<T> { fn apply<R, F: fn(T) -> R>(self, f: F) -> R { f(self.value) } }
fn probe() -> Int {
    let stage: Stage<Payload> = Stage { value: Payload { value: 7 } };
    let output: Payload = stage.apply(|x| x);
    output.value
}
"#,
    );
    let entry = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n.ends_with(".probe"))
        })
        .expect("entry")
        .id;
    let result = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
        .execute_function(entry)
        .expect("execute callback");
    assert_eq!(result.as_i64(), 7);
}

#[test]
fn callable_parameter_identity_survives_serialized_declaration_and_shadow() {
    let module = compile(
        r#"
type Payload is { value: Int };
type Stage<F> is { value: F };
implement<F> Stage<F> { fn transform<F: fn(Payload) -> Payload>(self, f: F) -> F { f } }
fn probe(stage: Stage<Int>) -> Int { let f = stage.transform(|x| x); 0 }
"#,
    );
    let module = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).expect("serialize"),
    )
    .expect("deserialize");
    let f = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n.ends_with("Stage.transform"))
        })
        .expect("method");
    assert_eq!(
        f.parameter_generic_id(1, |id| module.get_string(id)),
        Some(verum_vbc::types::TypeParamId(0x8000))
    );
    let payload = nominal(&module, "Payload");
    let function = TypeRef::Function {
        params: vec![payload.clone()],
        return_type: Box::new(payload),
        contexts: Default::default(),
    };
    assert_eq!(
        method_calls(&module, "Stage.transform").as_slice(),
        &[vec![TypeRef::Concrete(TypeId::INT), function]]
    );
}

#[test]
fn empty_parameter_name_cannot_alias_nonempty_string_slot_zero() {
    use verum_vbc::types::{StringId, TypeParamId};
    let mut f = FunctionDescriptor::default();
    f.params.push(verum_vbc::module::ParamDescriptor {
        name: StringId(1),
        type_ref: TypeRef::Concrete(TypeId::INT),
        is_mut: false,
        default: None,
        type_name: StringId::EMPTY,
    });
    f.type_params.push(verum_vbc::types::TypeParamDescriptor {
        name: StringId(1),
        id: TypeParamId(7),
        ..Default::default()
    });
    assert_eq!(f.parameter_generic_id(0, |_| Some("F")), None);
}

#[test]
fn sibling_callback_values_bind_distinct_return_and_callable_slots() {
    let module = compile(
        r#"
type Payload is { value: Int };
type Other is { value: Int };
type Stage<T> is { value: T };
implement<T> Stage<T> { fn transform<R, F: fn(T) -> R>(self, f: F) -> F { f } }
fn probe(stage: Stage<Payload>) -> Int {
    let first = stage.transform(|x| x);
    let second = stage.transform(|x| -> Other { Other { value: 3 } });
    0
}
"#,
    );
    let payload = nominal(&module, "Payload");
    let other = nominal(&module, "Other");
    assert_eq!(
        method_calls(&module, "Stage.transform").as_slice(),
        &[
            vec![
                payload.clone(),
                payload.clone(),
                TypeRef::Function {
                    params: vec![payload.clone()],
                    return_type: Box::new(payload.clone()),
                    contexts: Default::default(),
                }
            ],
            vec![
                payload.clone(),
                other.clone(),
                TypeRef::Function {
                    params: vec![payload],
                    return_type: Box::new(other),
                    contexts: Default::default(),
                }
            ],
        ]
    );
}

#[test]
fn ambiguous_legacy_parameter_names_do_not_invent_scope_ownership() {
    use verum_vbc::types::{StringId, TypeParamId};
    let mut f = FunctionDescriptor::default();
    f.params.push(verum_vbc::module::ParamDescriptor {
        name: StringId(2),
        type_ref: TypeRef::Concrete(TypeId::INT),
        is_mut: false,
        default: None,
        type_name: StringId(1),
    });
    for id in [TypeParamId(0), TypeParamId(0x8000)] {
        f.type_params.push(verum_vbc::types::TypeParamDescriptor {
            name: StringId(1),
            id,
            ..Default::default()
        });
    }
    assert_eq!(f.parameter_generic_id(0, |_| Some("F")), None);
    f.explicit_type_param_ids.push(Some(TypeParamId(0x8000)));
    assert_eq!(
        f.parameter_generic_id(0, |_| Some("F")),
        Some(TypeParamId(0x8000))
    );
}

#[test]
fn async_callable_is_not_certified_as_an_ordinary_function() {
    let module = compile(
        r#"
type Stage<T> is { value: T };
implement<T> Stage<T> { fn transform<F>(self, f: F) -> F { f } }
fn probe(stage: Stage<Int>) -> Int { let f = stage.transform(async |x: Int| x); 0 }
"#,
    );
    assert_eq!(
        method_calls(&module, "Stage.transform")[0][1],
        TypeRef::Generic(verum_vbc::types::TypeParamId(1))
    );
}

#[test]
fn self_annotation_keeps_existing_abi_type_even_without_callable_proof() {
    let module = compile(
        r#"
type Payload is { value: Int };
implement Payload { fn probe(self) -> Int { let f = |x: Self| -> Self { x }; 0 } }
"#,
    );
    let f = closures(&module)[0];
    assert_eq!(f.params[1].type_ref, nominal(&module, "Payload"));
    assert_eq!(f.return_type, nominal(&module, "Payload"));
}

#[test]
fn callable_identity_survives_the_following_generic_method() {
    let module = compile(
        r#"
type Payload is { value: Int };
type Stage<T> is { value: T };
type Wrapped<I, F> is { owner: I, callback: F };
implement<T> Stage<T> {
    fn transform<R, F: fn(T) -> R>(self, f: F) -> Wrapped<Self, F> { Wrapped { owner: self, callback: f } }
}
implement<I, F> Wrapped<I, F> { fn finish<C>(self) -> C { C.default() } }
fn probe(stage: Stage<Payload>) -> Int { stage.transform(|x| x).finish<Int>() }
"#,
    );
    let payload = nominal(&module, "Payload");
    let TypeRef::Concrete(stage) = nominal(&module, "Stage") else {
        panic!("stage")
    };
    let expected = vec![
        TypeRef::Instantiated {
            base: stage,
            args: vec![payload.clone()],
        },
        TypeRef::Function {
            params: vec![payload.clone()],
            return_type: Box::new(payload),
            contexts: Default::default(),
        },
        TypeRef::Concrete(TypeId::INT),
    ];
    assert_eq!(
        method_calls(&module, "Wrapped.finish").as_slice(),
        &[expected]
    );
}

#[test]
fn callable_identity_survives_a_binding_and_multiple_method_results() {
    let module = compile(
        r#"
type Payload is { value: Int };
type Stage<T> is { value: T };
type Wrapped<I, F> is { owner: I, callback: F };
implement<T> Stage<T> {
    fn transform<R, F: fn(T) -> R>(self, f: F) -> Wrapped<Self, F> { Wrapped { owner: self, callback: f } }
}
implement<I, F> Wrapped<I, F> {
    fn keep(self) -> Self { self }
    fn finish<C>(self) -> C { C.default() }
}
fn probe(stage: Stage<Payload>) -> Int {
    let mapped = stage.transform(|x| x).keep();
    mapped.finish<Int>()
}
"#,
    );
    let payload = nominal(&module, "Payload");
    let expected = TypeRef::Function {
        params: vec![payload.clone()],
        return_type: Box::new(payload),
        contexts: Default::default(),
    };
    assert_eq!(method_calls(&module, "Wrapped.keep")[0][1], expected);
    assert_eq!(method_calls(&module, "Wrapped.finish")[0][1], expected);
}

#[test]
fn unknown_callable_in_chain_cannot_bind_an_unrelated_nominal_f() {
    let module = compile(
        r#"
type F is { decoy: Int };
type Stage<T> is { value: T };
type Wrapped<I, F> is { owner: I, callback: F };
implement<T> Stage<T> {
    fn transform<F>(self, f: F) -> Wrapped<Self, F> { Wrapped { owner: self, callback: f } }
}
implement<I, F> Wrapped<I, F> { fn keep(self) -> Self { self } fn finish<C>(self) -> C { C.default() } }
fn probe(stage: Stage<Int>) -> Int { stage.transform(|x| x).keep().keep().finish<Int>() }
"#,
    );
    assert_eq!(
        method_calls(&module, "Wrapped.finish")[0][1],
        TypeRef::Generic(verum_vbc::types::TypeParamId(1))
    );
}

#[test]
fn method_shadow_callable_keeps_its_owner_binding_in_the_result() {
    let module = compile(
        r#"
type Payload is { value: Int };
type Stage<F> is { value: F };
type Wrapped<I, F> is { owner: I, callback: F };
implement<F> Stage<F> {
    fn transform<F: fn(Payload) -> Payload>(self, f: F) -> Wrapped<Self, F> { Wrapped { owner: self, callback: f } }
}
implement<I, F> Wrapped<I, F> { fn finish<C>(self) -> C { C.default() } }
fn probe(stage: Stage<Int>) -> Int { stage.transform(|x| x).finish<Int>() }
"#,
    );
    let payload = nominal(&module, "Payload");
    let TypeRef::Concrete(stage) = nominal(&module, "Stage") else {
        panic!("stage")
    };
    assert_eq!(
        method_calls(&module, "Wrapped.finish")[0],
        vec![
            TypeRef::Instantiated {
                base: stage,
                args: vec![TypeRef::Concrete(TypeId::INT)]
            },
            TypeRef::Function {
                params: vec![payload.clone()],
                return_type: Box::new(payload),
                contexts: Default::default()
            },
            TypeRef::Concrete(TypeId::INT),
        ]
    );
}

#[test]
fn static_callable_result_uses_parameter_zero_in_the_following_chain() {
    let module = compile(
        r#"
type Payload is { value: Int };
type Factory is { value: Int };
type Wrapped<I, F> is { owner: I, callback: F };
implement Factory {
    fn make<F: fn(Payload) -> Payload>(f: F) -> Wrapped<Payload, F> {
        Wrapped { owner: Payload { value: 7 }, callback: f }
    }
}
implement<I, F> Wrapped<I, F> { fn finish<C>(self) -> C { C.default() } }
fn probe() -> Int { Factory.make(|x| x).finish<Int>() }
"#,
    );
    let payload = nominal(&module, "Payload");
    assert_eq!(
        method_calls(&module, "Wrapped.finish")[0],
        vec![
            payload.clone(),
            TypeRef::Function {
                params: vec![payload.clone()],
                return_type: Box::new(payload),
                contexts: Default::default()
            },
            TypeRef::Concrete(TypeId::INT),
        ]
    );
}

#[test]
fn overlong_explicit_roster_does_not_partially_certify_a_callable_result() {
    // Public VBC lowering can be invoked without the source checker. An
    // invalid explicit roster must not become a partial semantic proof here.
    let module = compile(
        r#"
type Payload is { value: Int };
type Stage<T> is { value: T };
type Wrapped<I, F> is { owner: I, callback: F };
implement<T> Stage<T> {
    fn transform<R, F: fn(T) -> R>(self, f: F) -> Wrapped<Self, F> { Wrapped { owner: self, callback: f } }
}
implement<I, F> Wrapped<I, F> { fn finish<C>(self) -> C { C.default() } }
fn probe(stage: Stage<Payload>) -> Int {
    stage.transform<Payload, _, Bool>(|x| x).finish<Int>()
}
"#,
    );
    assert_eq!(
        method_calls(&module, "Wrapped.finish")[0][1],
        TypeRef::Generic(verum_vbc::types::TypeParamId(1))
    );
}
