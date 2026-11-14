//! A source wrapper must retain only the selected operation's proven footprint.
use verum_ast::{FileId, ItemKind};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::TypeChecker;

fn errors(consumer: &str, providers: &[(&str, &str)]) -> List<Text> {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog("demo");
    checker.set_current_module_path("demo.main");
    let mut registry = ModuleRegistry::new();
    for (index, (owner, source)) in providers
        .iter()
        .chain(std::iter::once(&("demo.main", consumer)))
        .enumerate()
    {
        let file_id = FileId::new(index as u32);
        let ast = FastParser::new()
            .parse_module_str(source, file_id)
            .expect("source grammar");
        let id = ModuleId::new(index as u32);
        let path = ModulePath::from_str(owner);
        let mut module = ModuleInfo::new(id, path.clone(), ast.clone(), file_id, (*source).into());
        module.exports = extract_exports_from_module(&ast, id, &path).expect("source exports");
        registry.register(module);
    }
    checker.set_module_registry_direct(registry.clone());
    let ast = FastParser::new()
        .parse_module_str(consumer, FileId::new(providers.len() as u32))
        .expect("consumer grammar");
    checker.register_stdlib_types_for_module(&ast);
    let mut errors = List::new();
    for item in &ast.items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(e) = checker.process_import(import, "demo.main", &registry) {
                errors.push(format!("{e:?}").into());
            }
        }
    }
    for item in &ast.items {
        let result = match &item.kind {
            ItemKind::Type(d) => checker.register_type_declaration(d),
            ItemKind::Impl(d) => checker.register_impl_block(d),
            ItemKind::Function(d) => checker.register_function_signature(d),
            ItemKind::Context(_) => checker.check_item(item),
            _ => continue,
        };
        if let Err(e) = result {
            errors.push(format!("{e:?}").into());
        }
    }
    for item in &ast.items {
        if matches!(item.kind, ItemKind::Context(_)) {
            continue;
        }
        if let Err(e) = checker.check_item(item) {
            errors.push(format!("{e:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|e| e.is_error())
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors
}

const PRIMITIVE: &str = r#"module demo.ops;
@intrinsic("text_byte_len") public fn byte_count(value: &Text) -> Int;
@intrinsic("text_as_bytes") public fn byte_view(value: &Text) -> &[Byte];
"#;

#[test]
fn imported_reference_forwarder_retains_its_operation_bound() {
    for (operation, result) in [("byte_count", "Int"), ("byte_view", "&[Byte]")] {
        for mounted in [operation.to_owned(), format!("{operation} as operation")] {
            let selected = if mounted.contains(" as ") { "operation" } else { operation };
            let api = format!("module demo.api; mount demo.ops.{{{mounted}}}; public fn observe(value: &Text) -> {result} {{ {selected}(value) }}");
            let consumer = "module demo.main; mount demo.api.observe; pure fn probe(value: &Text) -> Bool { let observed = observe(value); true }";
            let e = errors(consumer, &[("demo.ops", PRIMITIVE), ("demo.api", &api)]);
            assert!(e.is_empty(), "{mounted}: {e:?}");
        }
    }
}

#[test]
fn imported_method_forwarder_retains_its_operation_bound() {
    let api = "module demo.api; mount demo.ops.byte_count; public type Source is (); implement Source { public fn observe(&self, value: &Text) -> Int { byte_count(value) } }";
    let e = errors("module demo.main; mount demo.api.Source; pure fn probe(source: &Source, value: &Text) -> Int { source.observe(value) }", &[("demo.ops", PRIMITIVE), ("demo.api", api)]);
    assert!(e.is_empty(), "{e:?}");
}

fn rejects_purity(consumer: &str, api: &str) {
    let e = errors(consumer, &[("demo.ops", PRIMITIVE), ("demo.api", api)]);
    assert!(e.iter().any(|e| e.contains("ImpurePureFunction")), "expected a computational-contract refusal: {e:?}");
    assert!(!e.iter().any(|e| e.contains("UnboundVariable") || e.contains("TypeNotFound")), "callee must resolve before refusal: {e:?}");
}

#[test]
fn a_forwarder_cannot_borrow_a_same_named_operation() {
    rejects_purity(
        "module demo.main; mount demo.api.observe; pure fn probe(value: &Text) -> Int { observe(value) }",
        "module demo.api; extern \"C\" { fn touch() -> Int; } fn byte_count(value: &Text) -> Int { unsafe { touch() } } public fn observe(value: &Text) -> Int { byte_count(value) }",
    );
}

#[test]
fn a_parameter_shadowing_the_intrinsic_callee_remains_unknown() {
    rejects_purity(
        "module demo.main; mount demo.api.observe; pure fn probe(callback: fn(&Text)->Int, value: &Text) -> Int { observe(callback, value) }",
        "module demo.api; mount demo.ops.byte_count; public fn observe(byte_count: fn(&Text)->Int, value: &Text) -> Int { byte_count(value) }",
    );
}

#[test]
fn forwarding_a_hidden_global_does_not_prove_purity() {
    rejects_purity(
        "module demo.main; mount demo.api.observe; pure fn probe() -> Int { observe() }",
        "module demo.api; mount demo.ops.byte_count; static mut GLOBAL: Text = \"abc\"; public fn observe() -> Int { byte_count(&GLOBAL) }",
    );
}

#[test]
fn an_extra_operation_cannot_disappear_from_a_forwarder() {
    rejects_purity(
        "module demo.main; mount demo.api.observe; pure fn probe(value: &Text) -> Int { observe(value) }",
        "module demo.api; mount demo.ops.byte_count; extern \"C\" { fn touch(); } public fn observe(value: &Text) -> Int { unsafe { touch(); } byte_count(value) }",
    );
}

#[test]
fn structural_views_have_a_finite_bound_without_borrowing_user_methods() {
    for (parameter, body) in [
        ("&[Byte]", "value.len()"),
        ("&[Byte]", "let pointer = value.as_ptr(); value.len()"),
        ("&mut [Byte]", "let pointer = value.as_mut_ptr(); value.len()"),
        ("&[Byte]", "let pointer = value.offset(1); value.len()"),
        ("&unsafe Byte", "if value.is_null() { 0 } else { 1 }"),
    ] {
        let source = format!("module demo.main; pure fn probe(value: {parameter}) -> Int {{ {body} }}");
        let e = errors(&source, &[]);
        assert!(e.is_empty(), "{parameter} {body}: {e:?}");
    }
    rejects_purity(
        "module demo.main; mount demo.api.Source; pure fn probe(value: &Source) -> Int { value.len() }",
        "module demo.api; public type Source is (); extern \"C\" { fn touch() -> Int; } implement Source { public fn len(&self) -> Int { unsafe { touch() } } }",
    );
}

#[test]
fn pure_structural_results_do_not_hide_argument_computation() {
    let source = "module demo.main; extern \"C\" { fn touch() -> Int; } pure fn probe(value: &[Byte]) -> Int { let pointer = value.offset(unsafe { touch() }); value.len() }";
    let e = errors(source, &[]);
    assert!(e.iter().any(|e| e.contains("ImpurePureFunction")), "{e:?}");
}

#[test]
fn an_owned_input_does_not_inherit_a_reference_forwarding_proof() {
    rejects_purity(
        "module demo.main; mount demo.api.{Source,observe}; pure fn probe(source: Source, value: &Text) -> Int { observe(source, value) }",
        "module demo.api; mount demo.ops.byte_count; public type Source is (); public fn observe(source: Source, value: &Text) -> Int { byte_count(value) }",
    );
}

#[test]
fn reference_adaptation_needs_its_own_computational_evidence() {
    rejects_purity(
        "module demo.main; mount demo.api.{Source,observe}; pure fn probe(source: &Source) -> Int { observe(source) }",
        "module demo.api; mount demo.ops.byte_count; public type Source is (); public fn observe(source: &Source) -> Int { byte_count(source) }",
    );
}

#[test]
fn a_precondition_does_not_disappear_from_a_forwarder() {
    rejects_purity(
        "module demo.main; mount demo.api.observe; pure fn probe(value: &Text) -> Int { observe(value) }",
        "module demo.api; mount demo.ops.byte_count; extern \"C\" { fn touch() -> Bool; } public fn observe(value: &Text) -> Int requires unsafe { touch() } { byte_count(value) }",
    );
}

#[test]
fn the_consumers_homonym_cannot_replace_the_defining_operation() {
    let api = "module demo.api; mount demo.ops.{byte_count as selected}; public fn observe(value: &Text) -> Int { selected(value) }";
    let consumer = "module demo.main; mount demo.api.observe; extern \"C\" { fn touch() -> Int; } fn selected(value: &Text) -> Int { unsafe { touch() } } pure fn probe(value: &Text) -> Int { observe(value) }";
    let e = errors(consumer, &[("demo.ops", PRIMITIVE), ("demo.api", api)]);
    assert!(e.is_empty(), "{e:?}");
}


const SLICE_PRIMITIVE: &str = r#"module demo.ops;
public fn cardinality<T>(value: &[T]) -> USize { @intrinsic("slice_len", value) }
"#;

#[test]
fn declared_generic_reference_forwarding_preserves_a_checked_scalar_cast() {
    let api = "module demo.api; mount demo.ops.cardinality; public fn observe<T>(value: &[T]) -> Int { cardinality(value) as Int }";
    let source = "module demo.main; mount demo.api.observe; pure fn probe(value: &[Byte]) -> Int { observe(value) }";
    let e = errors(source, &[("demo.ops", SLICE_PRIMITIVE), ("demo.api", api)]);
    assert!(e.is_empty(), "{e:?}");
}

#[test]
fn a_receiver_keeps_its_declared_generic_reference_scope() {
    let api = "module demo.api; mount demo.ops.cardinality; public type Source<T> is { value: T }; implement<T> Source<T> { public fn observe(&self, value: &[T]) -> Int { cardinality(value) as Int } }";
    let source = "module demo.main; mount demo.api.Source; pure fn probe(source: &Source<Byte>, value: &[Byte]) -> Int { source.observe(value) }";
    let e = errors(source, &[("demo.ops", SLICE_PRIMITIVE), ("demo.api", api)]);
    assert!(e.is_empty(), "{e:?}");
}

#[test]
fn a_generic_reference_call_cannot_change_its_operand_shape() {
    let api = "module demo.api; mount demo.ops.cardinality; public fn observe<T>(value: &T) -> Int { cardinality(value) as Int }";
    let source = "module demo.main; mount demo.api.observe; pure fn probe(value: &Byte) -> Int { observe(value) }";
    let e = errors(source, &[("demo.ops", SLICE_PRIMITIVE), ("demo.api", api)]);
    assert!(e.iter().any(|e| e.contains("ImpurePureFunction")), "{e:?}");
}

#[test]
fn an_unbound_reference_placeholder_has_no_declaration_proof() {
    let api = "module demo.api; mount demo.ops.cardinality; public fn observe(value: &_) -> Int { cardinality(value) as Int }";
    let source = "module demo.main; mount demo.api.observe; pure fn probe(value: &[Byte]) -> Int { observe(value) }";
    let e = errors(source, &[("demo.ops", SLICE_PRIMITIVE), ("demo.api", api)]);
    assert!(e.iter().any(|e| e.contains("ImpurePureFunction")), "{e:?}");
}

#[test]
fn a_foreign_scalar_spelling_cannot_authorize_a_cast() {
    let ops = "module demo.ops; public type USize is (); public fn cardinality<T>(value: &[T]) -> USize { @intrinsic(\"slice_len\", value) }";
    let api = "module demo.api; mount demo.ops.cardinality; public fn observe<T>(value: &[T]) -> Int { cardinality(value) as Int }";
    let source = "module demo.main; mount demo.api.observe; pure fn probe(value: &[Byte]) -> Int { observe(value) }";
    let e = errors(source, &[("demo.ops", ops), ("demo.api", api)]);
    assert!(e.iter().any(|e| e.contains("ImpurePureFunction")), "{e:?}");
}

#[test]
fn an_intrinsic_leaf_cannot_hide_an_extra_computation() {
    let ops = "module demo.ops; extern \"C\" { fn touch(); } public fn cardinality<T>(value: &[T]) -> USize { unsafe { touch(); } @intrinsic(\"slice_len\", value) }";
    let api = "module demo.api; mount demo.ops.cardinality; public fn observe<T>(value: &[T]) -> Int { cardinality(value) as Int }";
    let source = "module demo.main; mount demo.api.observe; pure fn probe(value: &[Byte]) -> Int { observe(value) }";
    let e = errors(source, &[("demo.ops", ops), ("demo.api", api)]);
    assert!(e.iter().any(|e| e.contains("ImpurePureFunction")), "{e:?}");
}
