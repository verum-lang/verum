//! Shared callable admission depends on its declared environment, not its ABI.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    module::{FunctionId, VbcModule},
    serialize::serialize_module,
    types::{TypeId, TypeParamId, TypeRef},
};

fn compile(source: &str, bootstrap: bool) -> VbcModule {
    let ast = Parser::new(source).parse_module().unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("callable_ownership"));
    let module = if bootstrap {
        codegen.collect_unit_declarations(&[&ast]).unwrap();
        codegen
            .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
            .unwrap();
        codegen.finalize_module_from_state().unwrap()
    } else {
        codegen.compile_module(&ast).unwrap()
    };
    deserialize_module(&serialize_module(&module).unwrap()).unwrap()
}
fn execute(module: VbcModule) -> Result<i64, String> {
    let id = module
        .find_function_by_name("callable_ownership.probe")
        .unwrap();
    Interpreter::new(Arc::new(module))
        .execute_function(id)
        .map(|v| v.as_i64())
        .map_err(|e| e.to_string())
}

#[test]
fn callable_records_cross_generic_result_boundaries() {
    let source = r#"
type Factory is { create: fn() -> Int };
type Outcome<T, E> is Good(T) | Bad(E);
implement<T, E> Outcome<T, E> {
    fn map_error<F, O: fn(E) -> F>(self, f: O) -> Outcome<T, F> {
        match self { Good(value) => Good(value), Bad(error) => Bad(f(error)) }
    }
}
fn make() -> Outcome<Factory, Int> { Good(Factory { create: || 17 }) }
fn probe() -> Int {
    let mapped = make().map_error(|_| "failure");
    match mapped { Good(factory) => { let call = factory.create; call() }, Bad(_) => 0 }
}
"#;
    for bootstrap in [false, true] {
        assert_eq!(execute(compile(source, bootstrap)).unwrap(), 17);
    }
}

#[test]
fn immutable_mutable_and_nested_environments_keep_content_declarations() {
    for source in [
        "fn probe() -> Int { let value: Int = 7; let f = || value; f() }",
        "fn probe() -> Int { let mut value: Int = 7; let f = || { value = value + 1; value }; f(); f() }",
        "fn probe() -> Int { let value: Int = 7; let outer = || { let inner = || value; inner() }; outer() }",
    ] {
        for bootstrap in [false, true] {
            let expected = if source.contains("let mut") { 9 } else { 7 };
            assert_eq!(execute(compile(source, bootstrap)).unwrap(), expected);
        }
    }
}

#[test]
fn owned_affine_and_linear_captures_refuse_before_callable_creation() {
    for discipline in ["affine", "linear"] {
        for mutable in ["", "mut "] {
            let source = format!(
                "type {discipline} Token is {{ value: Int }}; fn probe() -> Int {{ let {mutable}token: Token = Token {{ value: 7 }}; let call = || token.value; call() }}"
            );
            for bootstrap in [false, true] {
                let error = execute(compile(&source, bootstrap)).unwrap_err();
                assert!(
                    error.contains("capture 0 requires unrestricted ownership"),
                    "{error}"
                );
                assert!(
                    error.contains(if discipline == "affine" {
                        "Affine"
                    } else {
                        "Linear"
                    }),
                    "{error}"
                );
            }
        }
    }
}

#[test]
fn borrowed_capture_keeps_its_reference_qualifier() {
    let source = "type affine Token is { value: Int }; fn probe() -> Int { let token: Token = Token { value: 7 }; let borrowed: &Token = &token; let call = || borrowed.value; call() }";
    for bootstrap in [false, true] {
        let module = compile(source, bootstrap);
        let body = verum_vbc::bytecode::decode_instructions(&module.bytecode).unwrap();
        assert!(body.iter().any(|i| matches!(i,
            verum_vbc::Instruction::NewClosure { capture_types: Some(ts), .. }
            if matches!(ts.as_slice(), [Some(TypeRef::Reference { .. })]))));
        assert_eq!(execute(module).unwrap(), 7);
    }
}

#[test]
fn missing_legacy_and_wrong_arity_capture_evidence_refuses() {
    for bootstrap in [false, true] {
        let original = compile(
            "fn probe() -> Int { let value: Int = 7; let f = || value; f() }",
            bootstrap,
        );
        for capture_types in [
            None,
            Some(vec![]),
            Some(vec![None]),
            Some(vec![Some(TypeRef::Concrete(TypeId(99999)))]),
        ] {
            let mut module = original.clone();
            rewrite(&mut module, |instruction| {
                if let verum_vbc::Instruction::NewClosure {
                    capture_types: types,
                    captures,
                    ..
                } = instruction
                {
                    if !captures.is_empty() {
                        *types = capture_types.clone();
                    }
                }
            });
            let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
            let error = execute(module).unwrap_err();
            assert!(error.contains("shared callable"), "{error}");
        }
    }
}

#[test]
fn creator_witnesses_and_exact_empty_arity_are_required() {
    let module = VbcModule::new("capture_check".into());
    let types = [Some(TypeRef::Generic(TypeParamId(7)))];
    assert!(
        module
            .validate_closure_captures(Some(&types), 0, Clone::clone)
            .unwrap_err()
            .contains("arity")
    );
    assert!(
        module
            .validate_closure_captures(Some(&types), 1, Clone::clone)
            .unwrap_err()
            .contains("Unknown")
    );
    module
        .validate_closure_captures(Some(&types), 1, |ty| match ty {
            TypeRef::Generic(TypeParamId(7)) => TypeRef::Concrete(TypeId::INT),
            _ => ty.clone(),
        })
        .unwrap();
    module
        .validate_closure_captures(None, 0, Clone::clone)
        .unwrap();
}

fn rewrite(module: &mut VbcModule, mut edit: impl FnMut(&mut verum_vbc::Instruction)) {
    let original = std::mem::take(&mut module.bytecode);
    for function in &mut module.functions {
        let range = function.bytecode_offset as usize
            ..(function.bytecode_offset + function.bytecode_length) as usize;
        let mut body = verum_vbc::bytecode::decode_instructions(&original[range]).unwrap();
        for instruction in &mut body {
            edit(instruction);
        }
        verum_vbc::bytecode::fixup_jump_offsets(&mut body);
        function.bytecode_offset = module.bytecode.len() as u32;
        for instruction in &body {
            verum_vbc::bytecode::encode_instruction(instruction, &mut module.bytecode);
        }
        function.bytecode_length = module.bytecode.len() as u32 - function.bytecode_offset;
        function.instructions = None;
    }
}

#[test]
fn creator_generic_capture_witnesses_survive_specialization() {
    let source =
        "fn make<T>(value: T) -> fn() -> T { || value } fn probe() -> Int { let f = make(7); f() }";
    for bootstrap in [false, true] {
        let mut module = compile(source, bootstrap);
        assert_eq!(execute(module.clone()).unwrap(), 7);
        let mut graph = verum_vbc::mono::InstantiationGraph::new();
        for function in &mut module.functions {
            function.is_generic = !function.type_params.is_empty();
        }
        for function in &module.functions {
            let range = function.bytecode_offset as usize
                ..(function.bytecode_offset + function.bytecode_length) as usize;
            let body = verum_vbc::bytecode::decode_instructions(&module.bytecode[range]).unwrap();
            verum_vbc::mono::discover_call_instantiations(
                &module,
                &body,
                function.func_id_base,
                &mut graph,
            )
            .unwrap();
        }
        assert!(!graph.is_empty());
        let specialized = verum_vbc::mono::monomorphize_minimal(module, &graph)
            .unwrap()
            .module;
        assert_eq!(execute(specialized).unwrap(), 7);
    }
}

#[test]
fn capture_declarations_remap_with_the_creation_instruction() {
    use verum_vbc::{
        Instruction,
        bytecode_remap::{IdRemap, rewrite_instruction_ids},
        instruction::Reg,
    };
    struct Relocate;
    impl IdRemap for Relocate {
        fn map_function(&self, id: FunctionId) -> FunctionId {
            FunctionId(id.0 + 10)
        }
        fn map_type_id(&self, id: TypeId) -> TypeId {
            TypeId(id.0 + 100)
        }
    }
    let mut instruction = Instruction::NewClosure {
        dst: Reg(3),
        func_id: 4,
        captures: vec![Reg(1)],
        capture_types: Some(vec![Some(TypeRef::Instantiated {
            base: TypeId(500),
            args: vec![TypeRef::Slice(Box::new(TypeRef::Concrete(TypeId(501))))],
        })]),
    };
    rewrite_instruction_ids(&mut instruction, &Relocate);
    let mut encoded = vec![];
    verum_vbc::bytecode::encode_instruction(&instruction, &mut encoded);
    let decoded = verum_vbc::bytecode::decode_instructions(&encoded).unwrap();
    assert!(
        matches!(&decoded[0], Instruction::NewClosure { func_id: 14, capture_types: Some(types), .. }
        if matches!(&types[0], Some(TypeRef::Instantiated { base: TypeId(600), args })
            if matches!(&args[0], TypeRef::Slice(inner)
                if **inner == TypeRef::Concrete(TypeId(601)))))
    );
    // The presence marker is part of the instruction, not a detached sidecar.
    let marker = 7; // extended opcode, dst, function, captures, count, slot count
    assert_eq!(encoded[marker], 1);
    encoded[marker] = 2;
    assert!(
        verum_vbc::bytecode::decode_instructions(&encoded)
            .unwrap_err()
            .to_string()
            .contains("capture type presence")
    );
}

#[test]
fn nested_generic_environment_does_not_invent_missing_lexical_witnesses() {
    let source = "fn make<T>(value: T) -> fn() -> T { || { let inner = || value; inner() } } fn probe() -> Int { let f = make(7); f() }";
    for bootstrap in [false, true] {
        let error = execute(compile(source, bootstrap)).unwrap_err();
        assert!(
            error.contains("capture 0 requires unrestricted ownership; found Unknown"),
            "{error}"
        );
    }
}

#[test]
fn environment_formation_does_not_invoke_a_declared_target() {
    use verum_vbc::{
        bytecode::{decode_instructions, encode_instructions},
        instruction::Instruction,
    };
    for bootstrap in [false, true] {
        for invoke in [false, true] {
            let tail = if invoke { "call()" } else { "7" };
            let source =
                format!("fn probe() -> Int {{ let value: Int = 7; let call = || value; {tail} }}");
            let mut module = compile(&source, bootstrap);
            assert!(module.functions.len() < 127);
            let mut instructions = decode_instructions(&module.bytecode).unwrap();
            let mut changed = 0;
            for instruction in &mut instructions {
                if let Instruction::NewClosure { func_id, .. } = instruction {
                    *func_id = 127;
                    changed += 1;
                }
            }
            assert_eq!(changed, 1);
            let original_length = module.bytecode.len();
            module.bytecode.clear();
            encode_instructions(&instructions, &mut module.bytecode);
            assert_eq!(
                module.bytecode.len(),
                original_length,
                "keep function offsets unchanged"
            );
            let result = execute(module);
            if invoke {
                assert_eq!(result.unwrap_err(), "Function 127 not found");
            } else {
                assert_eq!(result.unwrap(), 7);
            }
        }
    }
}
