#![cfg(feature = "codegen")]
//! Execute declaration-owned destruction against compiled source constructors and
//! user Drop bodies. Only the disposal boundary is an explicit bytecode fixture;
//! select lowering has its own integration tests.
#[path = "fixtures/canonical_drop.rs"]
mod canonical_drop;

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
mount core.base.protocols.Drop;
type Maybe<T> is None | Some(T);
type Guard is { drops: &mut Int, marker: Int };
implement Drop for Guard { fn drop(&mut self) { *self.drops = *self.drops + 1; } }
fn dispose<T>(value: T) {}
"#;

fn compile(body: &str, bootstrap: bool) -> VbcModule {
    compile_in("drop_runtime", body, bootstrap)
}

fn compile_in(owner: &str, body: &str, bootstrap: bool) -> VbcModule {
    try_compile_in(owner, body, bootstrap).unwrap_or_else(|error| panic!("{owner}: {error:?}"))
}

fn try_compile_in(owner: &str, body: &str, bootstrap: bool) -> Result<VbcModule, verum_vbc::codegen::CodegenError> {
    let source = format!("module {owner};\n{DECLARATIONS}\n{body}");
    let ast = Parser::new(&source).parse_module().expect("source grammar");
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new(owner));
    canonical_drop::initialize(&mut compiler);
    let mut module = if bootstrap {
        compiler
            .collect_unit_declarations(&[&ast])
            .expect("declarations");
        compiler.compile_unit_items(&[&ast], ItemFailurePolicy::Strict)?;
        compiler.finalize_module_from_state().expect("finalize")
    } else {
        compiler.compile_additional_module(&ast)?
    };
    let dispose = module
        .find_function_by_name(&format!("{owner}.dispose"))
        .expect("dispose declaration");
    assert!(emit_parameter_drop(&mut module, dispose).is_generic(),
        "disposal must exercise frame witness resolution");
    // The declaration and typed operand must survive a real archive boundary.
    Ok(deserialize_module(&serialize_module(&module).expect("serialize")).expect("deserialize"))
}

fn emit_parameter_drop(module: &mut VbcModule, entry: verum_vbc::module::FunctionId) -> verum_vbc::types::TypeRef {
    let declared = module.get_function(entry).unwrap().semantic_parameter_types()
        .expect("source parameter evidence")[0].clone().expect("declared disposal operand");
    let start = module.bytecode.len() as u32;
    encode_instruction(
        &Instruction::DropTyped {
            src: Reg(0),
            type_ref: declared.clone(),
        },
        &mut module.bytecode,
    );
    encode_instruction(&Instruction::RetV, &mut module.bytecode);
    let end = module.bytecode.len() as u32;
    let function = module
        .functions
        .iter_mut()
        .find(|function| function.id == entry)
        .unwrap();
    function.bytecode_offset = start;
    function.bytecode_length = end - start;
    function.instructions = None;
    declared
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
fn missing_drop_witness_identifies_the_type_and_source_frame() {
    for bootstrap in [false, true] {
        let module = compile("", bootstrap);
        let dispose = module.find_function_by_name("drop_runtime.dispose").unwrap();
        let error = Interpreter::new(Arc::new(module))
            .execute_function_with_args(dispose, &[verum_vbc::Value::from_i64(42)])
            .expect_err("a runtime value cannot supply the generic destructor witness");
        let message = error.to_string();
        assert!(message.contains("unresolved declaration witnesses for T0"), "{message}");
        assert!(message.contains("drop_runtime.dispose"), "{message}");
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
fn discarded_call_results_retire_the_selected_owned_payload() {
    run(
        r#"
fn make(drops: &mut Int) -> Maybe<Guard> {
    Maybe.Some(Guard { drops: drops, marker: 7 })
}
type Factory is {};
implement Factory {
    fn make(&self, drops: &mut Int) -> Maybe<Guard> { make(drops) }
}
fn borrow<T>(value: &T) -> &T { value }
fn probe() -> Int {
    let mut drops = 0;
    let factory = Factory {};
    make(&mut drops);
    if drops == 1 { factory.make(&mut drops) } else { make(&mut drops) };
    let owner = Guard { drops: &mut drops, marker: 9 };
    borrow(&owner);
    drops
}
"#,
        2,
    );
}

#[test]
fn discarded_standard_sum_runs_only_its_active_payload_destructor() {
    let parser = verum_fast_parser::FastParser::new();
    let mut maybe = parser.parse_module_str(include_str!("../../../core/base/maybe.vr"), verum_ast::FileId::new(1)).unwrap();
    maybe.items.retain(|item| matches!(&item.kind, verum_ast::ItemKind::Module(m) if m.items.is_none())
        || matches!(&item.kind, verum_ast::ItemKind::Type(decl) if decl.name.name == "Maybe"));
    let source = parser.parse_module_str(r#"
        module consumer;
        mount core.base.maybe.Maybe;
        mount core.base.protocols.Drop;
        type affine Guard is { drops: &mut Int };
        implement Drop for Guard { fn drop(&mut self) { *self.drops = *self.drops + 1; } }
        fn acquire(drops: &mut Int) -> Maybe<Guard> { Maybe.Some(Guard { drops: drops }) }
        fn empty() -> Maybe<Guard> { Maybe.None }
        fn probe() -> Int {
            let mut drops = 0;
            acquire(&mut drops);
            empty();
            drops
        }
    "#, verum_ast::FileId::new(2)).unwrap();
    let mut compiler = VbcCodegen::new();
    canonical_drop::initialize(&mut compiler);
    let units = [&maybe, &source];
    compiler.collect_unit_declarations(&units).unwrap();
    compiler.resolve_pending_imports();
    compiler.compile_unit_items(&units, ItemFailurePolicy::Strict).unwrap();
    let module = compiler.finalize_module().unwrap();
    let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    let entry = module.find_function_by_name("consumer.probe").unwrap();
    let value = Interpreter::try_new_validated(Arc::new(module)).unwrap().execute_function(entry).unwrap();
    assert!(value.is_int());
    assert_eq!(value.as_i64(), 1);
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
implement<T: Drop> Drop for Envelope<T> { fn drop(&mut self) { self.value.drop(); } }
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
        let mut module = compile(
            r#"
            type Map<K, V> is { len: Int, cap: Int, entries: &unsafe V, tombstones: Int };
            fn corruptible(value: Map<Int, Int>) { dispose<Map<Int, Int>>(value); }
            fn direct(value: Map<Int, Int>) {}
        "#,
            bootstrap,
        );
        let entry = module
            .find_function_by_name("drop_runtime.corruptible")
            .unwrap();
        let direct = module.find_function_by_name("drop_runtime.direct").unwrap();
        emit_parameter_drop(&mut module, direct);
        let mut interpreter = Interpreter::new(Arc::new(module));
        let foreign = interpreter
            .state
            .heap
            .alloc(verum_vbc::types::TypeId::MAP, 32)
            .unwrap();
        for (entry, reason) in [
            (entry, "typed copy nominal owner differs from its declaration"),
            (direct, "nominal owner mismatch"),
        ] {
            let error = interpreter.execute_function_with_args(
                entry, &[verum_vbc::Value::from_ptr(foreign.as_ptr())],
            ).unwrap_err();
            assert!(error.to_string().contains(reason), "bootstrap={bootstrap}: {error}");
        }
    }
}

#[test]
fn built_in_list_cleanup_requires_its_element_witness_and_visits_nested_owners() {
    use verum_vbc::{drop_plan, types::{TypeId, TypeRef}};
    run(r#"
        type Digit is { counter: &mut Int, digit: Int };
        implement Drop for Digit {
            fn drop(&mut self) { *self.counter = *self.counter * 10 + self.digit; }
        }
        type Basket is { items: List<Digit> };
        type Shipment is { baskets: List<Basket> };
        fn probe() -> Int {
            let mut digits = 0;
            {
                let shipment = Shipment { baskets: [
                    Basket { items: [Digit { counter: &mut digits, digit: 3 }] },
                    Basket { items: [Digit { counter: &mut digits, digit: 7 }] }
                ] };
            }
            digits
        }
    "#, 73);
    let module = VbcModule::new("bootstrap".into());
    for ty in [
        TypeRef::Concrete(TypeId::LIST),
        TypeRef::Instantiated { base: TypeId::LIST, args: vec![TypeRef::Concrete(TypeId(98765))] },
        TypeRef::Instantiated { base: TypeId(98765), args: vec![TypeRef::Concrete(TypeId::INT)] },
        TypeRef::Instantiated { base: TypeId::BYTE_LIST, args: vec![TypeRef::Concrete(TypeId::BYTE)] },
    ] {
        assert!(drop_plan::build(&module, &ty).is_err(), "{ty:?}");
    }
}

#[test]
fn a_list_storage_producer_executes_the_selected_source_drop_body() {
    let source = r#"
        type List<T> is { len: Int, cap: Int, ptr: &unsafe T };
        static mut OBSERVED: Int = 0;
        implement<T> Drop for List<T> {
            fn drop(&mut self) { unsafe { OBSERVED = self.len; } }
        }
        type Basket is { items: List<Int> };
        fn probe() -> Int {
            { let basket = Basket { items: [4, 9] }; }
            unsafe { OBSERVED }
        }
    "#;
    for bootstrap in [false, true] {
        let module = compile_in("core.collections.list", source, bootstrap);
        let entry = module.find_function_by_name("core.collections.list.probe").unwrap();
        let mut vm = Interpreter::try_new_validated(Arc::new(module)).unwrap();
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(2));
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
        fn drop_numbers(xs: List<Int>) {}
    "#;
    for bootstrap in [false, true] {
        let owner = "core.collections.list";
        let mut module = compile_in(owner, SOURCE, bootstrap);
        let entry = module
            .find_function_by_name(&format!("{owner}.ordered"))
            .unwrap();
        let numbers = module
            .find_function_by_name(&format!("{owner}.drop_numbers"))
            .unwrap();
        emit_parameter_drop(&mut module, numbers);
        let mut interpreter = Interpreter::new(Arc::new(module));
        let result = interpreter.execute_function(entry);
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
        let foreign = "example.foreign";
        let invalid_element = "type List<T> is { len: Int, cap: Int, ptr: &unsafe T };
            fn invalid() { dispose<List<List<Int>>>([[1], [true]]); }";
        assert!(try_compile_in(owner, invalid_element, bootstrap).is_err(),
            "a matching outer container cannot validate an incompatible inner element");
        let rejected = try_compile_in(foreign, SOURCE, bootstrap).err()
            .expect("a foreign same-leaf declaration cannot construct a native list literal");
        assert!(rejected.to_string().contains("no declared array construction"), "{rejected}");
        let mut module = compile_in(foreign,
            "type List<T> is { len: Int, cap: Int, ptr: &unsafe T }; fn direct(xs: List<Int>) {}", bootstrap);
        let entry = module.find_function_by_name("example.foreign.direct").unwrap();
        emit_parameter_drop(&mut module, entry);
        let mut interpreter = Interpreter::new(Arc::new(module));
        let actual_list = interpreter.alloc_list(&[verum_vbc::Value::from_i64(1)]).unwrap();
        let rejected = interpreter.execute_function_with_args(entry, &[actual_list]).unwrap_err();
        assert!(rejected.to_string().contains("nominal owner mismatch"), "{rejected}");
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

#[test]
fn transparent_generic_wrapper_destroys_its_actual_payload_carrier_once() {
    run(
        r#"
        type Wrapped<T> is (T);
        fn probe() -> Int {
            let mut drops = 0;
            dispose<Wrapped<Guard>>(Wrapped(Guard { drops: &mut drops, marker: 41 }));
            drops
        }
        "#,
        1,
    );
}

#[test]
fn transparent_scalar_has_no_fictitious_object_destructor() {
    run(
        "type Number is (Int); fn probe()->Int {dispose<Number>(Number(37));37}",
        37,
    );
}

#[test]
fn fixed_array_destruction_preserves_its_extent_and_element_obligations() {
    use verum_vbc::{drop_plan::{build, DropPlan}, types::{TypeId, TypeParamId, TypeRef}};
    let module = compile("", true);
    for length in [0, 1, 8, u64::MAX] {
        for element in [TypeId::INT, TypeId::U8, TypeId::U64] {
            let array = TypeRef::Array { element: Box::new(TypeRef::Concrete(element)), length };
            assert!(matches!(build(&module, &array).unwrap(), DropPlan::Ignore));
            let nested = TypeRef::Array { element: Box::new(array), length };
            assert!(matches!(build(&module, &nested).unwrap(), DropPlan::Ignore));
        }
        let guard = module.types.iter().find(|ty| module.get_string(ty.name) == Some("Guard")).unwrap().id;
        let array = TypeRef::Array { element: Box::new(TypeRef::Concrete(guard)), length };
        assert!(matches!(build(&module, &array).unwrap(), DropPlan::SequenceStorage {
            fixed_length: Some(n), element, ..
        } if n == length && matches!(*element, DropPlan::Glue { owner, .. } if owner == guard)));
        let unresolved = TypeRef::Array { element: Box::new(TypeRef::Generic(TypeParamId(0))), length };
        assert!(build(&module, &unresolved).unwrap_err().contains("unresolved declaration witnesses"));
    }
}

#[test]
fn a_const_generic_packed_field_survives_declared_owner_cleanup() {
    run(r#"
        type Buffer<const N: Int> is { data: [Byte; N] };
        implement<const N: Int> Buffer<N> {
            fn new()->Self { Self { data: [0; N] } }
            fn stride(&self)->Int {
                (&self.data[3] as *const Byte as Int) - (&self.data[0] as *const Byte as Int)
            }
        }
        fn probe()->Int { let buffer = Buffer<8>.new(); buffer.stride() }
    "#, 3);
}

#[test]
fn owned_fixed_array_runs_each_element_destructor_once() {
    run(r#"
        fn probe() -> Int {
            let mut a = 0; let mut b = 0;
            dispose<[Guard;2]>([Guard { drops:&mut a, marker:1 }, Guard { drops:&mut b, marker:2 }]);
            10*a + b
        }
    "#, 11);
}


#[test]
fn runtime_extent_retires_the_initialized_elements_once() {
    run(r#"
        fn retire<T>(values: [T; _]) -> Int { values.len() }
        fn probe() -> Int {
            let mut a = 0; let mut b = 0;
            let n = retire<Guard>([Guard { drops:&mut a, marker:1 }, Guard { drops:&mut b, marker:2 }]);
            100*n + 10*a + b
        }
    "#, 211);
}

#[test]
fn owned_array_rejects_a_wrong_extent_before_detaching_its_elements() {
    let source = r#"
        type Item is { n: Int };
        implement Drop for Item { fn drop(&mut self) { self.n = 0; } }
        fn make() -> [Item; 1] { [Item { n: 7 }] }
        fn one(items: [Item; 1]) {}
        fn two(items: [Item; 2]) {}
    "#;
    for bootstrap in [false, true] {
        let mut module = compile(source, bootstrap);
        let make = module.find_function_by_name("drop_runtime.make").unwrap();
        let one = module.find_function_by_name("drop_runtime.one").unwrap();
        let two = module.find_function_by_name("drop_runtime.two").unwrap();
        emit_parameter_drop(&mut module, one);
        emit_parameter_drop(&mut module, two);
        let mut vm = Interpreter::try_new_validated(Arc::new(module)).unwrap();
        let short = vm.execute_function(make).unwrap();
        let error = vm.execute_function_with_args(two, &[short]).unwrap_err();
        assert!(error.to_string().contains("extent"), "{error}");
        vm.execute_function_with_args(one, &[short])
            .expect("failed extent check left the original owner intact");
    }
}
