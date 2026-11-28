//! Prepared T1713 source controls; the preparing agent has not executed them.
//! Every type/protocol/impl is parsed from source. No Heap spelling, seeded
//! field maps, synthetic protocol implementations or no-op optional receiver
//! can establish the positive path. Setup must pass before the probe runs.

use verum_ast::{FileId, ItemKind, Module};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::TypeChecker;

const OWNER: &str = "core.field_protocol";
const DECLARATIONS: &str = r#"
    public type Packet is { public visible: Int, hidden: Int, public id: Bool, name: Bool };
"#;

const DEREF_TYPES: &str = r#"
    type Deref is protocol {
        type Target;
        fn deref(&self) -> &Self.Target;
    };
    type DerefMut is protocol extends Deref {
        fn deref_mut(&mut self) -> &mut Self.Target;
    };
    type FieldGuard<Noise, Item> is { noise: Noise, value: Item };
"#;
const DEREF_IMPLS: &str = r#"
    implement<Noise, Item> Deref for FieldGuard<Noise, Item> {
        type Target = Item;
        fn deref(&self) -> &Item { &self.value }
    }
    implement<Noise, Item> DerefMut for FieldGuard<Noise, Item> {
        fn deref_mut(&mut self) -> &mut Item { &mut self.value }
    }
"#;

const MAYBE: &str = "type Maybe<T> is None | Some(T);";

// Fallible is deliberately neither Result nor Maybe. Its Output is supplied
// by the parsed Try implementation, not the well-known-name fallback.
const TRY: &str = r#"
    type ControlFlow<B, C> is Break(B) | Continue(C);
    type Try is protocol {
        type Output;
        type Residual;
        fn from_output(output: Self.Output) -> Self;
        fn branch(self) -> ControlFlow<Self.Residual, Self.Output>;
    };
    type Fallible<T> is Failed(Int) | Passed(T);
    implement<T> Try for Fallible<T> {
        type Output = T;
        type Residual = Int;
        fn from_output(output: T) -> Fallible<T> { Passed(output) }
        fn branch(self) -> ControlFlow<Int, T> {
            match self {
                Failed(error) => Break(error),
                Passed(value) => Continue(value),
            }
        }
    }
"#;

fn parse(source: &str, file: u32) -> Module {
    FastParser::new()
        .parse_module_str(source, FileId::new(file))
        .expect("fixture follows the production grammar")
}

fn collect_diagnostics(checker: &mut TypeChecker, errors: &mut List<Text>) {
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
}

fn check(helpers: &str, probe: &str) -> List<Text> {
    check_with_declarations(DECLARATIONS, helpers, probe)
}

fn check_with_declarations(declarations: &str, helpers: &str, probe: &str) -> List<Text> {
    let source = format!("mount {OWNER}.{{Packet}}; {helpers} {probe}");
    let mut registry = ModuleRegistry::new();
    for (index, (owner, text)) in [(OWNER, declarations), ("consumer", source.as_str())]
        .into_iter()
        .enumerate()
    {
        let file = FileId::new(index as u32);
        let id = ModuleId::new(index as u32);
        let ast = parse(text, index as u32);
        let path = ModulePath::from_str(owner);
        let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), file, text.into());
        info.exports = extract_exports_from_module(&ast, id, &path).expect("fixture exports");
        registry.register(info);
    }
    let ast = parse(&source, 1);
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    checker.set_module_registry_direct(registry.clone());
    checker.register_stdlib_types_for_module(&ast);
    let mut setup_errors = List::new();
    for item in &ast.items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(error) = checker.process_import(import, "consumer", &registry) {
                setup_errors.push(Text::from(format!("{error:?}")));
            }
        }
    }
    for item in &ast.items {
        let result = match &item.kind {
            ItemKind::Type(declaration) => checker.register_type_declaration(declaration),
            ItemKind::Impl(declaration) => checker.register_impl_block(declaration),
            ItemKind::Function(declaration) => checker.register_function_signature(declaration),
            _ => continue,
        };
        if let Err(error) = result {
            setup_errors.push(Text::from(format!("{error:?}")));
        }
    }
    for item in &ast.items {
        if matches!(&item.kind, ItemKind::Function(declaration) if declaration.name.name.as_str() == "probe")
        {
            continue;
        }
        if let Err(error) = checker.check_item(item) {
            setup_errors.push(Text::from(format!("{error:?}")));
        }
    }
    collect_diagnostics(&mut checker, &mut setup_errors);
    assert!(
        setup_errors.is_empty(),
        "the declared protocol/impl/owner setup must pass independently: {setup_errors:?}"
    );
    let probe = ast
        .items
        .iter()
        .find(|item| matches!(&item.kind, ItemKind::Function(declaration) if declaration.name.name.as_str() == "probe"))
        .expect("exactly named probe body");
    let mut errors = List::new();
    if let Err(error) = checker.check_item(probe) {
        errors.push(Text::from(format!("{error:?}")));
    }
    collect_diagnostics(&mut checker, &mut errors);
    errors
}

fn assert_public(helpers: &str, probe: &str) {
    let errors = check(helpers, probe);
    assert!(
        errors.is_empty(),
        "declared public target field: {errors:?}"
    );
}

fn assert_private(helpers: &str, probe: &str) {
    let errors = check(helpers, probe);
    let is_target_refusal = |error: &Text| {
        error.contains("VisibilityError")
            && error.contains("name: Text { inner: \"hidden\" }")
            && error.contains("visibility: Text { inner: \"private\" }")
            && error.contains("module_path: Text { inner: \"core.field_protocol\" }")
    };
    assert!(
        !errors.is_empty() && errors.iter().all(is_target_refusal),
        "refuse the target's private field, not an unknown wrapper/protocol: {errors:?}"
    );
}

#[test]
fn declared_deref_public_target_read_is_allowed() {
    assert_public(
        &format!("{DEREF_TYPES} {DEREF_IMPLS}"),
        "fn probe(value: FieldGuard<Bool, Packet>) -> Int { value.visible }",
    );
}

#[test]
fn declared_deref_private_target_read_is_refused() {
    assert_private(
        &format!("{DEREF_TYPES} {DEREF_IMPLS}"),
        "fn probe(value: FieldGuard<Bool, Packet>) -> Int { value.hidden }",
    );
}

#[test]
fn sum_with_declared_deref_uses_the_target_field_type_and_visibility() {
    let helpers = format!(
        r#"{DEREF_TYPES}
        type EitherSide<Item> is LeftSide(Item) | RightSide(Item);
        type Payload<Value> is {{ item: Value }};
        implement<Item> Deref for EitherSide<Item> {{
            type Target = Item;
            fn deref(&self) -> &Item {{
                match self {{ LeftSide(ref v) => v, RightSide(ref v) => v }}
            }}
        }}"#
    );
    assert_public(&helpers, "fn probe(value: EitherSide<Packet>) -> Int { value.visible }");
    assert_private(&helpers, "fn probe(value: EitherSide<Packet>) -> Int { value.hidden }");
    let errors = check(&helpers, "fn probe(value: EitherSide<Packet>) -> Bool { value.visible }");
    assert!(errors.iter().any(|error| error.contains("Mismatch")), "{errors:?}");
    assert_public(&helpers, "fn probe(value: EitherSide<Payload<Bool>>) -> Bool { value.item }");
    let errors = check(&helpers, "fn probe(value: EitherSide<Payload<Bool>>) -> Int { value.item }");
    assert!(errors.iter().any(|error| error.contains("Mismatch")), "{errors:?}");
}

#[test]
fn sum_without_deref_rejects_payload_fields_before_matching() {
    let helpers = "type EitherSide<Item> is LeftSide(Item) | RightSide(Item);";
    for receiver in ["EitherSide<Packet>", "&EitherSide<Packet>"] {
        let probe = format!("fn probe(value: {receiver}) -> Int {{ value.visible }}");
        let errors = check(helpers, &probe);
        assert!(errors.iter().any(|error| error.contains("UnknownField")
            && error.contains("visible")), "{receiver}: {errors:?}");
    }
}

#[test]
fn declared_deref_mut_public_target_write_is_allowed() {
    assert_public(
        &format!("{DEREF_TYPES} {DEREF_IMPLS}"),
        "fn probe(value: &mut FieldGuard<Bool, Packet>) { value.visible = 7; }",
    );
}

#[test]
fn declared_deref_mut_private_target_write_is_refused() {
    assert_private(
        &format!("{DEREF_TYPES} {DEREF_IMPLS}"),
        "fn probe(value: &mut FieldGuard<Bool, Packet>) { value.hidden = 7; }",
    );
}

#[test]
fn two_declared_deref_hops_keep_public_target_field() {
    assert_public(
        &format!("{DEREF_TYPES} {DEREF_IMPLS}"),
        "fn probe(value: FieldGuard<Int, FieldGuard<Bool, Packet>>) -> Int { value.visible }",
    );
}

#[test]
fn reflected_spelling_resolves_a_dereferenced_value_field() {
    let helpers = format!("{DEREF_TYPES} {DEREF_IMPLS}");
    for receiver in ["FieldGuard<Int, Packet>", "FieldGuard<Int, FieldGuard<Bool, Packet>>"] {
        assert_public(&helpers, &format!("fn probe(value: {receiver}) -> Bool {{ value.id }}"));
        let private = check(&helpers, &format!("fn probe(value: {receiver}) -> Text {{ value.name }}"));
        assert!(private.iter().any(|e| e.contains("VisibilityError") && e.contains(OWNER)), "{private:?}");
        let wrong = check(&helpers, &format!("fn probe(value: {receiver}) -> UInt64 {{ value.id }}"));
        assert!(wrong.iter().any(|e| e.contains("Bool")), "{wrong:?}");
    }
}

#[test]
fn missing_value_field_cannot_be_supplied_by_type_metadata() {
    let helpers = format!("{DEREF_TYPES} {DEREF_IMPLS}");
    for receiver in ["Packet", "FieldGuard<Int, Packet>"] {
        let errors = check(&helpers, &format!("fn probe(value: {receiver}) -> Int {{ value.size }}"));
        assert!(errors.iter().any(|e| e.contains("size")), "{receiver}: {errors:?}");
    }
}

#[test]
fn two_declared_deref_hops_keep_private_target_owner() {
    assert_private(
        &format!("{DEREF_TYPES} {DEREF_IMPLS}"),
        "fn probe(value: FieldGuard<Int, FieldGuard<Bool, Packet>>) -> Int { value.hidden }",
    );
}

#[test]
fn own_field_wins_before_declared_deref_target() {
    let helpers = format!(
        r#"{DEREF_TYPES} {DEREF_IMPLS}
        type ShadowGuard<Item> is {{ hidden: Bool, value: Item }};
        implement<Item> Deref for ShadowGuard<Item> {{
            type Target = Item;
            fn deref(&self) -> &Item {{ &self.value }}
        }}
        "#
    );
    assert_public(
        &helpers,
        "fn probe(value: ShadowGuard<Packet>) -> Bool { value.hidden }",
    );
}

#[test]
fn wrapper_without_deref_does_not_expose_inner_fields() {
    let errors = check(
        DEREF_TYPES,
        "fn probe(value: FieldGuard<Bool, Packet>) -> Int { value.visible }",
    );
    assert!(
        errors.iter().any(|error| error.contains("visible")),
        "the positive path must require the parsed Deref implementation: {errors:?}"
    );
}

#[test]
fn declared_maybe_public_inner_field_is_allowed() {
    assert_public(
        MAYBE,
        "fn probe(value: Maybe<Packet>) -> Maybe<Int> { value?.visible }",
    );
}

#[test]
fn declared_maybe_private_inner_field_is_refused() {
    assert_private(
        MAYBE,
        "fn probe(value: Maybe<Packet>) -> Maybe<Int> { value?.hidden }",
    );
}

#[test]
fn declared_maybe_public_reference_inner_field_is_allowed() {
    assert_public(
        MAYBE,
        "fn probe(value: Maybe<&Packet>) -> Maybe<Int> { value?.visible }",
    );
}

#[test]
fn declared_maybe_private_reference_inner_field_is_refused() {
    assert_private(
        MAYBE,
        "fn probe(value: Maybe<&Packet>) -> Maybe<Int> { value?.hidden }",
    );
}

#[test]
fn optional_methods_and_nested_fields_keep_declared_result_owner() {
    let helpers = format!(r#"{MAYBE}
        type Local is {{ value: Int, nested: Maybe<Int> }};
        implement Local {{
            fn amount(&self) -> Int {{ self.value }}
            fn optional(&self) -> Maybe<Int> {{ self.nested }}
        }}
    "#);
    for expression in ["value?.amount()", "value?.optional()", "value?.nested"] {
        assert_public(&helpers, &format!(
            "fn probe(value: Maybe<Local>) -> Maybe<Int> {{ {expression} }}"
        ));
    }
}

#[test]
fn optional_field_result_uses_the_imported_family_owner() {
    let declarations = format!("{DECLARATIONS} public type Maybe<T> is None | Some(T);");
    let errors = check_with_declarations(&declarations,
        &format!("mount {OWNER}.{{Maybe}};"),
        "fn probe(value: Maybe<Packet>) -> Maybe<Int> { value?.visible }",
    );
    assert!(errors.is_empty(), "declaring module identity: {errors:?}");
}

#[test]
fn optional_field_result_keeps_serialized_nominal_family_identity() {
    use std::sync::Arc;
    use verum_types::core_metadata::*;
    let mut metadata = CoreMetadata { version: CORE_METADATA_VERSION, ..Default::default() };
    let key: Text = format!("{OWNER}.Maybe").into();
    metadata.type_declaration_order.push(key.clone());
    metadata.declaration_type_bindings.insert(OWNER.into(), Default::default());
    metadata.types.insert(key, TypeDescriptor {
        name: "Maybe".into(), module_path: OWNER.into(), origin_module_path: None,
        source_declaration: None,
        generic_params: List::from_single(GenericParam {
            name: "T".into(), bounds: List::new(), default: None,
            type_bounds: List::new(), pid: Some(0),
        }),
        kind: TypeDescriptorKind::Variant { cases: List::from_iter([
            VariantCase { name: "None".into(), payload: None },
            VariantCase { name: "Some".into(),
                payload: Some(VariantPayload::Tuple(List::from_single("T".into()))) },
        ]) },
        size: None, alignment: None, methods: List::new(), implements: List::new(),
        decl_span: None, is_public: true, is_transparent_wrapper: false,
        resource_discipline: verum_common::ResourceDiscipline::Unrestricted,
    });
    // This exercises the metadata consumer and wire codec, not a full bake.
    let metadata = Arc::new(decode_metadata(&encode_metadata(&metadata).unwrap()).unwrap());
    for eager in [false, true] {
        let mut checker = if eager { TypeChecker::new_with_core_eager(metadata.clone()) }
            else { TypeChecker::new_with_core(metadata.clone()) };
        checker.register_primitives();
        checker.set_current_module_path("consumer");
        let module = parse(&format!(r#"
            mount {OWNER}.Maybe;
            type Local is {{ value: Int }};
            fn probe(value: Maybe<Local>) -> Maybe<Int> {{ value?.value }}
        "#), 3);
        let mut errors: List<Text> = checker.prepare_module_declarations(&module)
            .into_iter().map(|error| format!("{error:?}").into()).collect();
        for item in &module.items {
            if let Err(error) = checker.check_item(item) { errors.push(format!("{error:?}").into()); }
        }
        collect_diagnostics(&mut checker, &mut errors);
        assert!(errors.is_empty(), "eager={eager}: {errors:?}");
    }
}

#[test]
fn a_record_named_maybe_cannot_supply_a_nullable_result() {
    let errors = check(
        "type Optional<T> is None | Some(T); type Maybe<T> is { payload: T };",
        "fn probe(value: Optional<Packet>) -> Maybe<Int> { value?.visible }",
    );
    assert!(errors.iter().any(|error| error.contains("E405")
        && error.contains("declared nullable result family")), "{errors:?}");
}

#[test]
fn a_record_named_maybe_cannot_supply_null_coalescing_payload() {
    let errors = check("type Maybe<T> is { payload: T };",
        "fn probe(value: Maybe<Int>) -> Int { value ?? 0 }",
    );
    assert!(errors.iter().any(|error| error.contains("Mismatch")
        && error.contains("Maybe")), "{errors:?}");
}

#[test]
fn declared_try_public_output_field_is_allowed() {
    assert_public(
        TRY,
        "fn probe(value: Fallible<Packet>) -> Int { try { value?.visible } recover { _ => 0 } }",
    );
}

#[test]
fn declared_try_private_output_field_is_refused() {
    assert_private(
        TRY,
        "fn probe(value: Fallible<Packet>) -> Int { try { value?.hidden } recover { _ => 0 } }",
    );
}

#[test]
fn declared_try_public_reference_output_field_is_allowed() {
    assert_public(
        TRY,
        "fn probe(value: Fallible<&Packet>) -> Int { try { value?.visible } recover { _ => 0 } }",
    );
}

#[test]
fn declared_try_private_reference_output_field_is_refused() {
    assert_private(
        TRY,
        "fn probe(value: Fallible<&Packet>) -> Int { try { value?.hidden } recover { _ => 0 } }",
    );
}
