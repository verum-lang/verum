//! T1753: explicit dereference copies a place at the existing value boundary.
//! Nested slots remain shallow; this is not a recursive snapshot contract.
#![cfg(feature = "codegen")]

use verum_common::{List, Set, Shared, Text};
use verum_fast_parser::Parser;
use verum_vbc::bytecode::decode_instructions;
use verum_vbc::codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen};
use verum_vbc::instruction::Instruction;
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::VbcModule;

// The first no-archive baseline is retained unchanged in its historical
// commits. Resource-copy positives need real declaration authority, not a
// manufactured unrestricted flag on a reserved runtime carrier.
fn resource_declaration(owner: &str, source: &str, names: &[&str]) -> VbcModule {
    let mut ast = Parser::new(source).parse_module().expect("production declaration grammar");
    ast.items.retain(|item| match &item.kind {
        verum_ast::ItemKind::Module(_) => true,
        verum_ast::ItemKind::Type(declaration) => names.iter().any(|name| *name == declaration.name.name.as_str()),
        _ => false,
    });
    let source = VbcCodegen::with_config(CodegenConfig::new(owner))
        .compile_module(&ast).expect("production declaration producer");
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&source).expect("declaration encode"),
    ).expect("declaration decode")
}

fn source_and_wire(body: &str) -> [(&'static str, VbcModule); 2] {
    let mut declarations = List::new();
    let mut mounts = Text::new();
    for (used, owner, source, names, name) in [
        (body.contains("List<"), "core.collections.list", include_str!("../../../core/collections/list.vr"), &["List"][..], "List"),
        (body.contains("Map<"), "core.collections.map", include_str!("../../../core/collections/map.vr"), &["Map", "Slot"][..], "Map"),
        (body.contains("Shared<"), "core.base.memory", include_str!("../../../core/base/memory.vr"), &["Shared", "SharedInner"][..], "Shared"),
    ] {
        if used {
            declarations.push(resource_declaration(owner, source, names));
            mounts.push_str(&format!("mount {owner}.{name};\n"));
        }
    }
    let source: Text = format!("module explicit_copy;\n{mounts}{body}").into();
    let parsed = Parser::new(source.as_str()).parse_module().expect("fixture grammar");
    let source = if declarations.is_empty() {
        VbcCodegen::with_config(CodegenConfig::new("explicit_copy"))
            .compile_module(&parsed).expect("source codegen")
    } else {
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("explicit_copy"));
        codegen.register_builtin_variants();
        codegen.register_stdlib_constants();
        codegen.register_stdlib_intrinsics();
        codegen.register_runtime_io_functions();
        let modules: List<_> = declarations.iter().collect();
        codegen.import_bootstrap_nominal_dependencies(&[&parsed], &modules)
            .expect("actual source-owned declaration import");
        codegen.collect_unit_declarations(&[&parsed]).expect("consumer declarations");
        codegen.compile_unit_items(&[&parsed], ItemFailurePolicy::Strict).expect("consumer bodies");
        codegen.finalize_module_from_state().expect("consumer module")
    };
    let bytes = verum_vbc::serialize::serialize_module(&source).expect("wire encode");
    let wire = verum_vbc::deserialize::deserialize_module(&bytes).expect("wire decode");
    [("source", source), ("wire", wire)]
}

fn operations(module: &VbcModule) -> List<Instruction> {
    let function = module.functions.iter()
        .find(|f| module.get_string(f.name) == Some("explicit_copy.probe"))
        .expect("exact source probe");
    let start = function.bytecode_offset as usize;
    decode_instructions(&module.bytecode[start..start + function.bytecode_length as usize])
        .expect("actual emitted body").into_iter().collect()
}

fn check(body: &str, expected: i64) {
    let mut failures: List<Text> = List::new();
    for (route, module) in source_and_wire(body) {
        let function = module.functions.iter()
            .find(|f| module.get_string(f.name) == Some("explicit_copy.probe"))
            .expect("exact source probe").id;
        let emitted = operations(&module);
        let result = Interpreter::new(Shared::new(module).into_arc()).execute_function(function);
        match result {
            Ok(actual) if actual.is_int() && actual.as_i64() == expected => {}
            actual => failures.push(format!("{route}: expected {expected}, got {actual:?}; emitted {emitted:?}").into()),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

const SIMPLE: &str = r#"
type Packet is { value: Int };
fn probe() -> Int {
    let mut owner = Packet { value: 7 };
    let borrowed = &owner;
    let mut copied = *borrowed;
    copied.value = 9;
    owner.value * 10 + copied.value
}
"#;

#[test]
fn dereference_copy_emits_clone_of_the_loaded_value() {
    for (route, module) in source_and_wire(SIMPLE) {
        let emitted = operations(&module);
        let mut loaded = Set::new();
        let mut copied = false;
        for instruction in &emitted {
            match instruction {
                Instruction::Deref { dst, .. } => { loaded.insert(*dst); }
                Instruction::Mov { dst, src } if loaded.contains(src) => { loaded.insert(*dst); }
                Instruction::Clone { src, .. } if loaded.contains(src) => { copied = true; }
                _ => {}
            }
        }
        assert!(copied, "{route}: dereferenced place has no value-copy boundary: {emitted:?}");
    }
}

#[test]
fn named_record_copy_already_owns_its_outer_slots() {
    check(r#"
type Packet is { value: Int };
fn probe() -> Int {
    let mut owner = Packet { value: 7 };
    let mut copied = owner;
    copied.value = 9;
    owner.value * 10 + copied.value
}
"#, 79);
}

#[test]
fn inferred_record_dereference_copy_owns_its_outer_slots() { check(SIMPLE, 79); }

#[test]
fn annotated_record_dereference_copy_owns_its_outer_slots() {
    check(r#"
type Packet is { value: Int };
fn probe() -> Int {
    let mut owner = Packet { value: 7 };
    let borrowed: &Packet = &owner;
    let mut copied: Packet = *borrowed;
    copied.value = 9;
    owner.value * 10 + copied.value
}
"#, 79);
}

#[test]
fn parentheses_preserve_the_dereferenced_place() {
    check(r#"
type Packet is { value: Int };
fn probe() -> Int {
    let mut owner = Packet { value: 7 };
    let borrowed = &owner;
    let mut copied: Packet = (*borrowed);
    copied.value = 9;
    owner.value * 10 + copied.value
}
"#, 79);
}

#[test]
fn returned_public_projection_copies_the_projected_record() {
    check(r#"
type Packet is { value: Int };
type Owner is { packet: Packet };
implement Owner { public fn packet(&self) -> &Packet { &self.packet } }
fn probe() -> Int {
    let owner = Owner { packet: Packet { value: 7 } };
    let mut copied: Packet = *owner.packet();
    copied.value = 9;
    owner.packet.value * 10 + copied.value
}
"#, 79);
}

#[test]
fn copying_a_reference_keeps_its_alias() {
    check(r#"
type Packet is { value: Int };
fn probe() -> Int {
    let mut owner = Packet { value: 7 };
    let borrowed: &mut Packet = &mut owner;
    let copied: &mut Packet = borrowed;
    copied.value = 9;
    owner.value * 10 + copied.value
}
"#, 99);
}

#[test]
fn dereferencing_a_reference_to_a_reference_keeps_the_inner_alias() {
    check(r#"
type Packet is { value: Int };
fn probe() -> Int {
    let mut owner = Packet { value: 7 };
    let borrowed: &mut Packet = &mut owner;
    let outer = &borrowed;
    let copied: &mut Packet = *outer;
    copied.value = 9;
    owner.value * 10 + copied.value
}
"#, 99);
}

#[test]
fn copied_record_keeps_nested_record_slots_shallow() {
    check(r#"
type Packet is { value: Int };
type Owner is { stamp: Int, packet: Packet };
fn probe() -> Int {
    let owner = Owner { stamp: 1, packet: Packet { value: 7 } };
    let borrowed = &owner;
    let mut copied: Owner = *borrowed;
    copied.stamp = 2;
    copied.packet.value = 9;
    owner.stamp * 1000 + copied.stamp * 100 + owner.packet.value * 10 + copied.packet.value
}
"#, 1299);
}

#[test]
fn copied_record_keeps_nested_list_slots_shallow() {
    check(r#"
type Owner is { stamp: Int, items: List<Int> };
fn probe() -> Int {
    let owner = Owner { stamp: 1, items: [7] };
    let borrowed = &owner;
    let mut copied: Owner = *borrowed;
    copied.stamp = 2;
    copied.items[0] = 9;
    owner.stamp * 1000 + copied.stamp * 100 + owner.items[0] * 10 + copied.items[0]
}
"#, 1299);
}

#[test]
fn copying_the_list_itself_gets_a_separate_spine() {
    check(r#"
fn probe() -> Int {
    let owner: List<Int> = [7];
    let borrowed = &owner;
    let mut copied: List<Int> = *borrowed;
    copied[0] = 9;
    copied.push(2);
    owner[0] * 1000 + owner.len() * 100 + copied[0] * 10 + copied.len()
}
"#, 7192);
}

#[test]
fn copied_record_keeps_nested_variant_slots_shallow() {
    check(r#"
type Packet is { value: Int };
type Choice is Item(Packet);
type Owner is { stamp: Int, item: Choice };
fn probe() -> Int {
    let owner = Owner { stamp: 1, item: Choice.Item(Packet { value: 7 }) };
    let borrowed = &owner;
    let mut copied: Owner = *borrowed;
    copied.stamp = 2;
    match &mut copied.item { Choice.Item(packet) => { packet.value = 9; } }
    let retained = match &owner.item { Choice.Item(packet) => packet.value };
    owner.stamp * 100 + copied.stamp * 10 + retained
}
"#, 129);
}

#[test]
fn copied_record_keeps_nested_map_slots_shallow() {
    check(r#"
type Owner is { stamp: Int, items: Map<Int, Int> };
fn probe() -> Int {
    let mut items: Map<Int, Int> = Map.new();
    items.insert(1, 7);
    let owner = Owner { stamp: 1, items };
    let borrowed = &owner;
    let mut copied: Owner = *borrowed;
    copied.stamp = 2;
    copied.items.insert(1, 9);
    let retained = match owner.items.get(&1) { Maybe.Some(value) => *value, Maybe.None => 0 };
    owner.stamp * 100 + copied.stamp * 10 + retained
}
"#, 129);
}


#[test]
fn assignment_copies_the_dereferenced_record() {
    check(r#"
type Packet is { value: Int };
fn probe() -> Int {
    let owner = Packet { value: 7 };
    let borrowed = &owner;
    let mut copied = Packet { value: 0 };
    copied = *borrowed;
    copied.value = 9;
    owner.value * 10 + copied.value
}
"#, 79);
}

#[test]
fn record_field_store_copies_the_dereferenced_record() {
    check(r#"
type Packet is { value: Int };
type Holder is { packet: Packet };
fn probe() -> Int {
    let owner = Packet { value: 7 };
    let borrowed = &owner;
    let mut copied = Holder { packet: *borrowed };
    copied.packet.value = 9;
    owner.value * 10 + copied.packet.value
}
"#, 79);
}

#[test]
fn container_insertion_copies_the_dereferenced_record() {
    check(r#"
type Packet is { value: Int };
fn probe() -> Int {
    let owner = Packet { value: 7 };
    let borrowed = &owner;
    let mut copied: List<Packet> = [];
    copied.push(*borrowed);
    copied[0].value = 9;
    owner.value * 10 + copied[0].value
}
"#, 79);
}

#[test]
fn parenthesized_temporary_is_not_copied() {
    for (route, module) in source_and_wire(r#"
type Packet is { value: Int };
fn fresh() -> Packet { Packet { value: 7 } }
fn probe() -> Int {
    let owned = (fresh());
    owned.value
}
"#) {
        assert!(!operations(&module).iter().any(|op| matches!(op, Instruction::Clone { .. })),
            "{route}: a temporary acquired a copy boundary");
        let function = module.functions.iter()
            .find(|f| module.get_string(f.name) == Some("explicit_copy.probe")).unwrap().id;
        assert_eq!(Interpreter::new(Shared::new(module).into_arc())
            .execute_function(function).expect("temporary execution").as_i64(), 7);
    }
}

// The interpreter's supported three-slot Shared carrier and its recorded live
// SharedInner extent are seeded at the public execution boundary. This tests
// emitted source/wire copies, not a replacement Shared constructor or methods.
fn shared_carrier_control(body: &str, copies: bool) {
    use verum_vbc::{interpreter::OBJECT_HEADER_SIZE, types::TypeId, value::Value};
    let with_release: Text = format!("{body}\nfn release(value: Shared<Int>) {{ drop(value); }}\n").into();
    for (route, module) in source_and_wire(&with_release) {
        assert_eq!(operations(&module).iter().any(|op| matches!(op, Instruction::Clone { .. })),
            copies, "{route}: Shared ownership-copy emission");
        let function = module.functions.iter()
            .find(|f| module.get_string(f.name) == Some("explicit_copy.probe")).unwrap().id;
        let release = module.functions.iter()
            .find(|f| module.get_string(f.name) == Some("explicit_copy.release")).unwrap().id;
        let emitted = operations(&module);
        let mut vm = Interpreter::new(Shared::new(module).into_arc());
        let mut inner = Box::new([Value::from_i64(1), Value::from_i64(0), Value::from_i64(7)]);
        let address = inner.as_mut_ptr() as usize;
        vm.state.cbgr_bridge_extents.insert(address, std::mem::size_of_val(inner.as_ref()));
        let carrier = vm.state.heap.alloc(TypeId::SHARED, 3 * std::mem::size_of::<Value>()).expect("carrier");
        let pointer = carrier.as_ptr().cast::<u8>();
        // SAFETY: the owned carrier contains exactly these three Value slots;
        // the boxed three-slot extent remains live through execution.
        unsafe {
            let fields = pointer.add(OBJECT_HEADER_SIZE).cast::<Value>();
            fields.write(Value::from_i64(address as i64));
            fields.add(1).write(Value::from_i64(1));
            fields.add(2).write(Value::from_i64(0));
        }
        let original = Value::from_ptr(pointer);
        let result = vm.execute_function_with_args(function, &[original]).expect("Shared execution");
        assert_eq!(result.bits(), original.bits(), "{route}: Shared must retain its carrier identity");
        assert_eq!(inner[0].as_i64(), if copies { 2 } else { 1 },
            "{route}: the returned copied owner remains live; {emitted:?}");
        if copies {
            vm.execute_function_with_args(release, &[result]).expect("release copied owner");
            assert_eq!(inner[0].as_i64(), 1, "{route}: releasing the returned copy leaves the original owner");
        }
        assert_eq!(inner[2].as_i64(), 7, "{route}: Shared payload is unchanged");
        vm.state.cbgr_bridge_extents.remove(&address);
    }
}

#[test]
fn dereference_copy_keeps_shared_identity_and_counted_ownership() {
    shared_carrier_control(r#"
fn probe(value: Shared<Int>) -> Shared<Int> {
    let borrowed = &value;
    let copied = *borrowed;
    copied
}
"#, true);
}

#[test]
fn reference_copy_does_not_acquire_shared_ownership() {
    shared_carrier_control(r#"
fn probe(value: Shared<Int>) -> Shared<Int> {
    let borrowed = &value;
    let copied = borrowed;
    *copied
}
"#, false);
}

#[test]
fn inferred_returned_reference_layers_keep_the_alias() {
    check(r#"
type Packet is { value: Int };
fn borrow(value: &mut Packet) -> &mut Packet { value }
fn probe() -> Int {
    let mut owner = Packet { value: 7 };
    let returned = borrow(&mut owner);
    let outer = &returned;
    let copied = *outer;
    copied.value = 9;
    owner.value * 10 + copied.value
}
"#, 99);
}

#[test]
fn declared_generic_method_reference_result_keeps_the_alias() {
    check(r#"
type Packet is { value: Int };
type Holder<T> is { value: T };
implement<T> Holder<T> { fn get(&self) -> &T { &self.value } }
fn probe() -> Int {
    let mut owner = Packet { value: 7 };
    let holder: Holder<&mut Packet> = Holder { value: &mut owner };
    let copied: &mut Packet = *holder.get();
    copied.value = 9;
    owner.value * 10 + copied.value
}
"#, 99);
}

#[test]
fn dereferencing_a_borrowed_reference_field_keeps_the_alias() {
    check(r#"
type Packet is { value: Int };
type Holder is { value: &mut Packet };
fn probe() -> Int {
    let mut owner = Packet { value: 7 };
    let holder = Holder { value: &mut owner };
    let outer = &holder.value;
    let copied = *outer;
    copied.value = 9;
    owner.value * 10 + copied.value
}
"#, 99);
}

#[test]
fn dereferencing_a_borrowed_reference_element_keeps_the_alias() {
    check(r#"
type Packet is { value: Int };
fn probe() -> Int {
    let mut owner = Packet { value: 7 };
    let items: List<&mut Packet> = [&mut owner];
    let outer = &items[0];
    let copied = *outer;
    copied.value = 9;
    owner.value * 10 + copied.value
}
"#, 99);
}

#[test]
fn generic_function_reference_result_keeps_the_alias() {
    check(r#"
type Packet is { value: Int };
fn identity<T>(value: T) -> T { value }
fn probe() -> Int {
    let mut owner = Packet { value: 7 };
    let returned = identity(&mut owner);
    let outer = &returned;
    let copied = *outer;
    copied.value = 9;
    owner.value * 10 + copied.value
}
"#, 99);
}

#[test]
fn parenthesized_affine_place_preserves_the_existing_transfer() {
    for initializer in ["original", "((original))"] {
        let source: Text = format!(r#"
type Count is {{ value: Int }};
type affine Watch is {{ counter: &mut Count }};
implement Drop for Watch {{ fn drop(&mut self) {{ self.counter.value += 1; }} }}
fn probe() -> Int {{
    let mut counter = Count {{ value: 0 }};
    {{
        let original: Watch = Watch {{ counter: &mut counter }};
        let moved = {initializer};
    }}
    counter.value
}}
"#).into();
        for (route, module) in source_and_wire(source.as_str()) {
            let emitted = operations(&module);
            assert!(!emitted.iter().any(|op| matches!(op, Instruction::Clone { .. })),
                "{route}/{initializer}: an affine transfer must not clone: {emitted:?}");
            assert!(emitted.iter().any(|op| matches!(op, Instruction::LoadUnit { .. })),
                "{route}/{initializer}: the transferred source was not cleared: {emitted:?}");
            let id = module.find_function_by_name("explicit_copy.probe").unwrap();
            assert_eq!(Interpreter::new(Shared::new(module).into_arc())
                .execute_function(id).expect("affine handoff").as_i64(), 1,
                "{route}/{initializer}: one destination owns cleanup");
        }
    }
}

#[test]
fn dereference_cannot_copy_an_affine_or_linear_owner() {
    for qualifier in ["affine", "linear"] {
        for wrapper in [false, true] {
            let source: Text = if wrapper {
                format!("module explicit_copy; type {qualifier} Token is {{ value: Int }}; type Holder is {{ token: Token }}; fn probe(original: &Holder) {{ let copied = *original; }}").into()
            } else {
                format!("module explicit_copy; type {qualifier} Token is {{ value: Int }}; fn probe(original: &Token) {{ let copied = *original; }}").into()
            };
            let ast = Parser::new(source.as_str()).parse_module().expect("resource grammar");
            let result = VbcCodegen::with_config(CodegenConfig::new("explicit_copy")).compile_module(&ast);
            let error = result.expect_err("a borrowed exclusive resource cannot acquire copy or transfer permission");
            assert!(format!("{error:?}").contains("dereference copy requires an unrestricted value"),
                "{qualifier}, wrapper={wrapper}: exact resource-copy refusal: {error:?}");
        }
    }
}

#[test]
fn reordered_impl_reference_result_keeps_its_actual_parameter_layer() {
    check(r#"
type Packet is { value: Int };
type Pair<A, B> is { first: A, second: B };
implement<A, B> Pair<B, A> { fn get(&self) -> &A { &self.second } }
fn probe() -> Int {
    let mut owner = Packet { value: 7 };
    let pair: Pair<Packet, &mut Packet> = Pair { first: Packet { value: 1 }, second: &mut owner };
    let copied = *pair.get();
    copied.value = 9;
    owner.value * 10 + copied.value
}
"#, 99);
}

#[test]
fn reordered_impl_owned_result_copies_its_actual_parameter() {
    check(r#"
type Packet is { value: Int };
type Pair<A, B> is { first: A, second: B };
implement<A, B> Pair<B, A> { fn get(&self) -> &A { &self.second } }
fn probe() -> Int {
    let mut owner = Packet { value: 1 };
    let pair: Pair<&mut Packet, Packet> = Pair { first: &mut owner, second: Packet { value: 7 } };
    let mut copied = *pair.get();
    copied.value = 9;
    pair.second.value * 10 + copied.value
}
"#, 79);
}

#[test]
fn mounted_same_leaf_callable_keeps_its_selected_reference_result() {
    const BORROW: &str = "module source { public fn select(value: &mut explicit_copy.Packet) -> &mut explicit_copy.Packet { value } }";
    const OWNED: &str = "module foreign { public fn select(value: &mut explicit_copy.Packet) -> explicit_copy.Packet { explicit_copy.Packet { value: 2 } } }";
    for reverse in [false, true] {
        let source: Text = format!(r#"
public type Packet is {{ value: Int }};
{} {}
mount source.{{select as chosen}};
fn probe() -> Int {{
    let mut owner = Packet {{ value: 7 }};
    let returned = chosen(&mut owner);
    let outer = &returned;
    let copied = *outer;
    copied.value = 9;
    owner.value * 10 + copied.value
}}
"#, if reverse { OWNED } else { BORROW }, if reverse { BORROW } else { OWNED }).into();
        check(source.as_str(), 99);
    }
}

#[test]
fn container_copy_authority_comes_from_real_source_declarations() {
    use verum_vbc::types::{TypeId, TypeRef};
    for (route, module) in source_and_wire("fn probe(a: List<Int>, b: Map<Int, Int>, c: Shared<Int>) -> Int { 7 }") {
        for (id, owner, args) in [
            (TypeId::LIST, "core.collections.list", 1),
            (TypeId::MAP, "core.collections.map", 2),
            (TypeId::SHARED, "core.base.memory", 1),
        ] {
            let declaration = module.get_type(id).expect("canonical declaration");
            assert_eq!(declaration.origin_module.and_then(|id| module.get_string(id)), Some(owner), "{route}");
            assert!(declaration.fields.iter().all(|field| field.declaration_type.is_some()), "{route}: original semantic fields");
            assert_eq!(module.resource_discipline(&TypeRef::Instantiated {
                base: id, args: (0..args).map(|_| TypeRef::Concrete(TypeId::INT)).collect(),
            }), verum_common::ResourceDiscipline::Unrestricted, "{route}: {owner}");
        }
    }
}
