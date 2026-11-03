#![cfg(feature = "codegen")]
//! Execute declaration-owned destruction against compiled source constructors and
//! user Drop bodies. Only the disposal boundary is an explicit bytecode fixture;
//! select lowering has its own integration tests.
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    Instruction,
    bytecode::encode_instruction,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    instruction::Reg,
    interpreter::Interpreter,
    module::VbcModule,
    serialize::serialize_module,
};

const DECLARATIONS: &str = r#"
type Maybe<T> is None | Some(T);
type Guard is { drops: &mut Int, marker: Int };
implement Drop for Guard { fn drop(&mut self) { *self.drops = *self.drops + 1; } }
fn dispose<T>(value: T) {}
"#;

fn compile(body: &str, bootstrap: bool) -> VbcModule {
    compile_in("drop_runtime", body, bootstrap)
}

fn compile_in(owner: &str, body: &str, bootstrap: bool) -> VbcModule {
    let source = format!("module {owner};\n{DECLARATIONS}\n{body}");
    let ast = Parser::new(&source).parse_module().expect("source grammar");
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new(owner));
    let mut module = if bootstrap {
        compiler
            .collect_unit_declarations(&[&ast])
            .expect("declarations");
        compiler
            .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
            .expect("items");
        compiler.finalize_module_from_state().expect("finalize")
    } else {
        compiler.compile_module(&ast).expect("source codegen")
    };
    let dispose = module
        .find_function_by_name(&format!("{owner}.dispose"))
        .expect("dispose declaration");
    let function = module.get_function(dispose).unwrap();
    let declared = function
        .semantic_parameter_types()
        .expect("source parameter evidence")[0]
        .clone()
        .expect("source generic declaration");
    assert!(
        declared.is_generic(),
        "disposal must exercise frame witness resolution"
    );
    let start = module.bytecode.len() as u32;
    encode_instruction(
        &Instruction::DropTyped {
            src: Reg(0),
            type_ref: declared,
        },
        &mut module.bytecode,
    );
    encode_instruction(&Instruction::RetV, &mut module.bytecode);
    let end = module.bytecode.len() as u32;
    let function = module
        .functions
        .iter_mut()
        .find(|function| function.id == dispose)
        .unwrap();
    function.bytecode_offset = start;
    function.bytecode_length = end - start;
    function.instructions = None;
    // The declaration and typed operand must survive a real archive boundary.
    deserialize_module(&serialize_module(&module).expect("serialize")).expect("deserialize")
}

fn run(body: &str, expected: i64) {
    for bootstrap in [false, true] {
        let module = compile(body, bootstrap);
        let entry = module.find_function_by_name("drop_runtime.probe").unwrap();
        let value = Interpreter::new(Arc::new(module))
            .execute_function(entry)
            .unwrap_or_else(|error| panic!("bootstrap={bootstrap}: {error}"));
        assert!(value.is_int(), "expected Int, got {value:?}");
        assert_eq!(value.as_i64(), expected, "bootstrap={bootstrap}");
    }
}

#[test]
fn generic_maybe_destroys_its_owned_payload_once() {
    run(
        r#"
fn probe() -> Int {
    let mut drops = 0;
    dispose<Maybe<Guard>>(Maybe.Some(Guard { drops: &mut drops, marker: 41 }));
    drops
}

"#,
        1,
    );
}

#[test]
fn empty_record_and_unit_have_valid_cleanup_without_payloads() {
    run(
        r#"
type Empty is {};
fn probe() -> Int {
    dispose<Empty>(Empty {});
    dispose<()>(());
    37
}
"#,
        37,
    );
}

#[test]
fn undeclared_runtime_sum_tag_refuses_destruction() {
    for bootstrap in [false, true] {
        let module = compile(
            "fn corruptible(value: Maybe<Guard>) { dispose<Maybe<Guard>>(value); }",
            bootstrap,
        );
        let entry = module
            .find_function_by_name("drop_runtime.corruptible")
            .unwrap();
        let owner = module
            .get_function(entry)
            .unwrap()
            .semantic_parameter_types()
            .unwrap()[0]
            .as_ref()
            .unwrap()
            .base_type_id()
            .unwrap();
        let mut interpreter = Interpreter::new(Arc::new(module));
        // A real tracked allocation with the correct owner and intact extent,
        // but an impossible tag. This reaches DropTyped, not constructor validation.
        let invalid = interpreter.state.heap.alloc_variant(owner, 99, 0).unwrap();
        let error = interpreter
            .execute_function_with_args(entry, &[verum_vbc::Value::from_ptr(invalid.as_ptr())])
            .unwrap_err();
        assert!(
            error.to_string().contains("sum tag is not declared"),
            "bootstrap={bootstrap}: {error}"
        );
    }
}

#[test]
fn nested_user_sum_and_record_follow_declared_owned_components() {
    run(
        r#"
type Packet<T> is Empty | Held(T);
type Envelope<T> is { padding: Int, value: T };
fn probe() -> Int {
    let mut drops = 0;
    dispose<Packet<Envelope<Maybe<Guard>>>>(Packet.Held(Envelope { padding: 99, value: Maybe.Some(Guard { drops: &mut drops, marker: 41 }) }));
    drops
}
"#,
        1,
    );
}

#[test]
fn generic_explicit_drop_receives_its_exact_frame_witness() {
    run(
        r#"
type Envelope<T> is { value: T };
implement<T> Drop for Envelope<T> { fn drop(&mut self) { self.value.drop(); } }
fn probe() -> Int {
    let mut drops = 0;
    dispose<Maybe<Envelope<Guard>>>(Maybe.Some(Envelope { value: Guard { drops: &mut drops, marker: 41 } }));
    drops
}
"#,
        1,
    );
}

#[test]
fn none_and_unselected_sum_payloads_do_not_run_destructors() {
    run(
        r#"
type Packet<T> is Empty | Number(Int) | Held(T);
fn probe() -> Int {
    let mut drops = 0;
    let empty: Maybe<Guard> = Maybe.None;
    let alternate: Packet<Guard> = Packet.Number(23);
    dispose<Maybe<Guard>>(empty);
    dispose<Packet<Guard>>(alternate);
    drops
}
"#,
        0,
    );
}

#[test]
fn borrowed_payload_remains_live_without_running_owned_drop() {
    run(
        r#"
fn probe() -> Int {
    let mut drops = 0;
    let guard = Guard { drops: &mut drops, marker: 41 };
    dispose<Maybe<&Guard>>(Maybe.Some(&guard));
    assert(drops == 0);
    guard.marker
}
"#,
        41,
    );
}

#[test]
fn raw_payload_remains_live_without_running_owned_drop() {
    run(
        r#"
fn probe() -> Int {
    let mut drops = 0;
    let guard = Guard { drops: &mut drops, marker: 41 };
    let raw = &guard as &unsafe Guard;
    dispose<Maybe<&unsafe Guard>>(Maybe.Some(raw));
    assert(drops == 0);
    guard.marker
}
"#,
        41,
    );
}

#[test]
fn missing_payload_declaration_refuses_execution_instead_of_guessing_ownership() {
    for bootstrap in [false, true] {
        let mut module = compile(
            r#"
fn probe() -> Int {
    let mut drops = 0;
    dispose<Maybe<Guard>>(Maybe.Some(Guard { drops: &mut drops, marker: 41 }));
    drops
}

"#,
            bootstrap,
        );
        let maybe = module
            .types
            .iter()
            .find(|ty| {
                module
                    .get_string(ty.name)
                    .is_some_and(|name| name == "Maybe" || name == "drop_runtime.Maybe")
            })
            .expect("source Maybe declaration")
            .id;
        let declaration = module.types.iter_mut().find(|ty| ty.id == maybe).unwrap();
        let payload = declaration
            .variants
            .iter_mut()
            .find(|variant| variant.arity == 1)
            .unwrap();
        assert_eq!(payload.fields.len(), 1);
        payload.fields[0].declaration_type = None;
        let entry = module.find_function_by_name("drop_runtime.probe").unwrap();
        let error = Interpreter::new(Arc::new(module))
            .execute_function(entry)
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("field lacks declaration evidence"),
            "bootstrap={bootstrap}: {error}"
        );
    }
}

#[test]
fn nominal_primitive_metadata_cannot_silently_authorize_discarding_owned_fields() {
    for bootstrap in [false, true] {
        let mut module = compile(
            "type Carrier is { value: Int }; fn probe() -> Int { 0 }",
            bootstrap,
        );
        let owner = module
            .types
            .iter()
            .find(|ty| {
                module
                    .get_string(ty.name)
                    .is_some_and(|name| name == "Carrier" || name == "drop_runtime.Carrier")
            })
            .expect("source record")
            .id;
        module
            .types
            .iter_mut()
            .find(|ty| ty.id == owner)
            .unwrap()
            .kind = verum_vbc::types::TypeKind::Primitive;
        let error =
            verum_vbc::drop_plan::build(&module, &verum_vbc::types::TypeRef::Concrete(owner))
                .expect_err("nonbuiltin primitive layout is not ownership evidence");
        assert!(
            error.contains("nominal primitive lacks an owned declaration"),
            "{error}"
        );
    }
}

#[test]
fn foreign_map_declaration_cannot_claim_the_vm_table() {
    for bootstrap in [false, true] {
        let module = compile(
            r#"
            type Map<K, V> is { len: Int, cap: Int, entries: &unsafe V, tombstones: Int };
            fn corruptible(value: Map<Int, Int>) { dispose<Map<Int, Int>>(value); }
        "#,
            bootstrap,
        );
        let entry = module
            .find_function_by_name("drop_runtime.corruptible")
            .unwrap();
        let mut interpreter = Interpreter::new(Arc::new(module));
        let foreign = interpreter
            .state
            .heap
            .alloc(verum_vbc::types::TypeId::MAP, 32)
            .unwrap();
        let error = interpreter
            .execute_function_with_args(entry, &[verum_vbc::Value::from_ptr(foreign.as_ptr())])
            .unwrap_err();
        assert!(
            error.to_string().contains("nominal owner mismatch"),
            "bootstrap={bootstrap}: {error}"
        );
    }
}

#[test]
fn native_list_destruction_uses_the_declared_element_and_owner() {
    const SOURCE: &str = r#"
        type List<T> is { len: Int, cap: Int, ptr: &unsafe T };
        type Digit is { counter: &mut Int, digit: Int };
        implement Drop for Digit {
            fn drop(&mut self) { *self.counter = *self.counter * 10 + self.digit; }
        }
        fn ordered() -> Int {
            let mut digits = 0;
            dispose<List<List<Digit>>>([
                [Digit { counter: &mut digits, digit: 1 }, Digit { counter: &mut digits, digit: 2 }],
                [Digit { counter: &mut digits, digit: 3 }],
            ]);
            digits
        }
        fn drop_numbers(xs: List<Int>) { dispose<List<Int>>(xs); }
    "#;
    for bootstrap in [false, true] {
        for owner in ["core.collections.list", "example.foreign"] {
            let module = compile_in(owner, SOURCE, bootstrap);
            let entry = module
                .find_function_by_name(&format!("{owner}.ordered"))
                .unwrap();
            let numbers = module
                .find_function_by_name(&format!("{owner}.drop_numbers"))
                .unwrap();
            let mut interpreter = Interpreter::new(Arc::new(module));
            let result = interpreter.execute_function(entry);
            if owner == "example.foreign" {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("nominal owner mismatch")
                );
                continue;
            }
            assert_eq!(
                result.expect("all three owned elements dropped").as_i64(),
                321
            );
            let packed = interpreter
                .state
                .heap
                .alloc(verum_vbc::types::TypeId::BYTE_LIST, 24)
                .unwrap();
            let error = interpreter
                .execute_function_with_args(numbers, &[verum_vbc::Value::from_ptr(packed.as_ptr())])
                .unwrap_err();
            assert!(
                error.to_string().contains("Byte element witness"),
                "{error}"
            );
        }
    }
}

#[test]
fn transparent_aliases_keep_owned_arguments_and_refuse_erased_targets() {
    use verum_vbc::{drop_plan::{build, DropPlan}, types::{TypeId, TypeRef}};
    let mut module = compile("type Collection<T> is Maybe<T>;", true);
    let id = |name: &str| module.types.iter().find(|ty| module.get_string(ty.name) == Some(name)).unwrap().id;
    let alias = id("Collection");
    let guard = id("Guard");
    let ty = TypeRef::Instantiated { base: alias, args: vec![TypeRef::Concrete(guard)] };
    let DropPlan::Sum { variants, .. } = build(&module, &ty).unwrap() else { panic!("alias lost the underlying sum"); };
    assert!(variants.iter().any(|(_, fields)| matches!(fields.as_slice(), [DropPlan::Glue { owner, .. }] if *owner == guard)));
    for target in [None, Some(TypeRef::Concrete(TypeId::UNIT)), Some(TypeRef::Concrete(TypeId::PTR))] {
        module.types.iter_mut().find(|ty| ty.id == alias).unwrap().alias_target = target;
        assert!(build(&module, &ty).is_err(), "erased alias must not authorize destruction");
    }
}
