//! T1733: sum payload substitution follows the resolved declaration's parameters.
//! Parsed source only: no synthetic parameter tables or metadata descriptors.

use verum_ast::{FileId, ItemKind};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::TypeChecker;

const ALPHA: &str = r#"
public type Packet is { public count: Int };
public type Outcome<Value, Failure, Phantom> is | Ok(Value) | Err(Failure);
public fn first() -> Outcome<Packet, Text, Bool> { Outcome.Ok(Packet { count: 3 }) }
"#;

const BETA: &str = r#"
public type Packet is { public flag: Bool };
public type Outcome<Phantom, Value, Failure> is | Ok(Value) | Err(Failure);
public fn second() -> Outcome<Int, Packet, Bool> { Outcome.Ok(Packet { flag: true }) }
"#;

const ALIASES: &str = r#"
public type Outcome<Phantom, Value, Failure> is | Ok(Value) | Err(Failure);
public type Reordered<Failure, Value = Int> is Outcome<Bool, Value, Failure>;
public fn integer() -> Reordered<Text> { Outcome.Ok(7) }
public fn message() -> Reordered<Bool, Text> { Outcome.Ok("message") }
"#;

fn check(consumer: &str, modules: &[(&str, &str)]) -> List<Text> {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog("demo");
    checker.set_current_module_path("demo.main");
    let mut registry = ModuleRegistry::new();
    for (index, (owner, source)) in modules
        .iter()
        .chain(std::iter::once(&("demo.main", consumer)))
        .enumerate()
    {
        let file_id = FileId::new(index as u32);
        let ast = FastParser::new()
            .parse_module_str(source, file_id)
            .expect("fixture follows the production grammar");
        let id = ModuleId::new(index as u32);
        let path = ModulePath::from_str(owner);
        let mut module = ModuleInfo::new(id, path.clone(), ast.clone(), file_id, (*source).into());
        module.exports = extract_exports_from_module(&ast, id, &path).expect("source exports");
        registry.register(module);
    }
    checker.set_module_registry_direct(registry.clone());
    let ast = FastParser::new()
        .parse_module_str(consumer, FileId::new(modules.len() as u32))
        .expect("consumer follows the production grammar");
    checker.register_stdlib_types_for_module(&ast);
    let mut errors = List::new();
    for item in &ast.items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(error) = checker.process_import(import, "demo.main", &registry) {
                errors.push(format!("{error:?}").into());
            }
        }
    }
    for item in &ast.items {
        let result = match &item.kind {
            ItemKind::Type(decl) => checker.register_type_declaration(decl),
            ItemKind::Function(decl) => checker.register_function_signature(decl),
            _ => continue,
        };
        if let Err(error) = result {
            errors.push(format!("{error:?}").into());
        }
    }
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
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
    errors
}

fn modules(reverse: bool) -> [(&'static str, &'static str); 2] {
    if reverse {
        [("demo.beta", BETA), ("demo.alpha", ALPHA)]
    } else {
        [("demo.alpha", ALPHA), ("demo.beta", BETA)]
    }
}

fn assert_mismatch(errors: &List<Text>, expected: &[&str]) {
    assert!(
        errors
            .iter()
            .any(|error| error.contains("Mismatch")
                && expected.iter().all(|name| error.contains(*name))),
        "expected a real payload mismatch containing {expected:?}: {errors:?}"
    );
}

fn renamed_source(reverse_imports: bool, reverse_matches: bool, separate: bool) -> Text {
    let imports = [
        "mount demo.alpha.{Outcome as Left, Packet as First, first};",
        "mount demo.beta.{Outcome as Right, Packet as Second, second};",
    ];
    let mut source = Text::new();
    for index in if reverse_imports { [1, 0] } else { [0, 1] } {
        source.push_str(imports[index]);
    }
    source.push_str(
        "fn take_first(value: First) {} fn take_second(value: Second) {} fn take_text(value: Text) {} fn take_bool(value: Bool) {}",
    );
    let matches = [
        "match first() { Left.Ok(value) => take_first(value), Left.Err(error) => take_text(error) };",
        "match second() { Right.Ok(value) => take_second(value), Right.Err(error) => take_bool(error) };",
    ];
    if !separate {
        source.push_str("fn probe() {");
    }
    for index in if reverse_matches { [1, 0] } else { [0, 1] } {
        if separate {
            source.push_str(&format!("fn probe{index}() {{"));
        }
        source.push_str(matches[index]);
        if separate {
            source.push_str("}");
        }
    }
    if !separate {
        source.push_str("}");
    }
    source
}

#[test]
fn renamed_sums_keep_reordered_and_phantom_slots_in_one_caller() {
    for reverse_modules in [false, true] {
        for reverse_imports in [false, true] {
            for reverse_matches in [false, true] {
                let source = renamed_source(reverse_imports, reverse_matches, false);
                let errors = check(&source, &modules(reverse_modules));
                assert!(
                    errors.is_empty(),
                    "modules={reverse_modules}, imports={reverse_imports}, matches={reverse_matches}: {errors:?}"
                );
            }
        }
    }
}

#[test]
fn renamed_sums_keep_reordered_and_phantom_slots_across_callers() {
    for reverse_modules in [false, true] {
        for reverse_imports in [false, true] {
            for reverse_matches in [false, true] {
                let source = renamed_source(reverse_imports, reverse_matches, true);
                let errors = check(&source, &modules(reverse_modules));
                assert!(
                    errors.is_empty(),
                    "modules={reverse_modules}, imports={reverse_imports}, matches={reverse_matches}: {errors:?}"
                );
            }
        }
    }
}

#[test]
fn renamed_sum_refuses_the_foreign_same_leaf_payload_owner() {
    for reverse in [false, true] {
        let source = renamed_source(reverse, reverse, false)
            .replace("take_second(value)", "take_first(value)");
        let errors = check(&source, &modules(reverse));
        assert_mismatch(&errors, &["Packet", "alpha", "beta"]);
    }
}

#[test]
fn renamed_sum_refuses_the_other_declarations_error_slot() {
    for reverse in [false, true] {
        let source = renamed_source(reverse, reverse, false).replace(
            "Right.Err(error) => take_bool(error)",
            "Right.Err(error) => take_text(error)",
        );
        let errors = check(&source, &modules(reverse));
        assert_mismatch(&errors, &["Bool", "Text"]);
    }
}

fn qualified_source(reverse: bool) -> Text {
    let mut source = Text::from(
        r#"
mount demo.alpha.{Packet as First};
mount demo.beta.{Packet as Second};
type Outcome<Only> is | Ambient(Only);
fn take_first(value: First) {}
fn take_second(value: Second) {}
fn take_text(value: Text) {}
fn take_bool(value: Bool) {}
fn take_int(value: Int) {}
fn ambient(value: Outcome<Int>) { match value { Outcome.Ambient(item) => take_int(item) }; }
"#,
    );
    let functions = [
        "fn alpha(value: demo.alpha.Outcome<First, Text, Bool>) { match value { demo.alpha.Outcome.Ok(item) => take_first(item), demo.alpha.Outcome.Err(error) => take_text(error) }; }",
        "fn beta(value: demo.beta.Outcome<Int, Second, Bool>) { match value { demo.beta.Outcome.Ok(item) => take_second(item), demo.beta.Outcome.Err(error) => take_bool(error) }; }",
    ];
    for index in if reverse { [1, 0] } else { [0, 1] } {
        source.push_str(functions[index]);
    }
    source
}

#[test]
fn qualified_sums_ignore_a_different_arity_bare_namesake() {
    for reverse_modules in [false, true] {
        for reverse_functions in [false, true] {
            let errors = check(
                &qualified_source(reverse_functions),
                &modules(reverse_modules),
            );
            assert!(
                errors.is_empty(),
                "modules={reverse_modules}, functions={reverse_functions}: {errors:?}"
            );
        }
    }
}

#[test]
fn qualified_sum_refuses_a_wrong_error_payload_with_a_bare_namesake() {
    for reverse in [false, true] {
        let source = qualified_source(reverse).replace(
            "demo.beta.Outcome.Err(error) => take_bool(error)",
            "demo.beta.Outcome.Err(error) => take_text(error)",
        );
        let errors = check(&source, &modules(reverse));
        assert_mismatch(&errors, &["Bool", "Text"]);
    }
}

fn alias_source(reverse: bool) -> Text {
    let mut source = Text::from(
        r#"
mount demo.aliases.{Outcome as Carrier, Reordered as Local, integer, message};
fn take_int(value: Int) {}
fn take_text(value: Text) {}
fn take_bool(value: Bool) {}
fn probe() {
"#,
    );
    let matches = [
        "match integer() { Carrier.Ok(value) => take_int(value), Carrier.Err(error) => take_text(error) };",
        "match message() { Carrier.Ok(value) => take_text(value), Carrier.Err(error) => take_bool(error) };",
    ];
    for index in if reverse { [1, 0] } else { [0, 1] } {
        source.push_str(matches[index]);
    }
    source.push_str("}");
    source
}

#[test]
fn generic_alias_default_and_reordered_target_keep_both_payloads() {
    for reverse in [false, true] {
        let errors = check(&alias_source(reverse), &[("demo.aliases", ALIASES)]);
        assert!(errors.is_empty(), "reverse={reverse}: {errors:?}");
    }
}

#[test]
fn mounted_alias_annotations_use_the_alias_default_before_target_slots() {
    let source = r#"
mount demo.aliases.{Outcome as Carrier, Reordered as Local};
fn take_int(value: Int) {}
fn take_text(value: Text) {}
fn take_bool(value: Bool) {}
fn defaulted(value: Local<Text>) {
    match value { Carrier.Ok(item) => take_int(item), Carrier.Err(error) => take_text(error) };
}
fn explicit(value: Local<Bool, Text>) {
    match value { Carrier.Ok(item) => take_text(item), Carrier.Err(error) => take_bool(error) };
}
"#;
    let errors = check(source, &[("demo.aliases", ALIASES)]);
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn generic_alias_refuses_a_wrong_success_payload() {
    for reverse in [false, true] {
        let source = alias_source(reverse).replace(
            "match message() { Carrier.Ok(value) => take_text(value)",
            "match message() { Carrier.Ok(value) => take_int(value)",
        );
        let errors = check(&source, &[("demo.aliases", ALIASES)]);
        assert_mismatch(&errors, &["Int", "Text"]);
    }
}

#[test]
fn generic_alias_refuses_a_wrong_defaulted_error_payload() {
    for reverse in [false, true] {
        let source = alias_source(reverse).replace(
            "match integer() { Carrier.Ok(value) => take_int(value), Carrier.Err(error) => take_text(error)",
            "match integer() { Carrier.Ok(value) => take_int(value), Carrier.Err(error) => take_bool(error)",
        );
        let errors = check(&source, &[("demo.aliases", ALIASES)]);
        assert_mismatch(&errors, &["Bool", "Text"]);
    }
}

const REFINED_SUM: &str = r#"
public type Sized<N: meta USize {it == 4}> is | Content(Int);
"#;

#[test]
fn qualified_meta_argument_uses_its_declared_refinement() {
    let source = r#"
type Sized<N: meta USize {it == 8}> is | Ambient(Int);
fn qualified(value: demo.widths.Sized<4>) {}
fn local(value: Sized<8>) {}
"#;
    let errors = check(source, &[("demo.widths", REFINED_SUM)]);
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn qualified_meta_argument_cannot_borrow_a_namesakes_looser_value() {
    let source = r#"
type Sized<N: meta USize {it == 8}> is | Ambient(Int);
fn local(value: Sized<8>) {}
fn refused(value: demo.widths.Sized<8>) {}
"#;
    let errors = check(source, &[("demo.widths", REFINED_SUM)]);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("MetaRefinementViolation")
                && error.contains("Sized")
                && error.contains("position: 1")
                && error.contains("\"8\"")
                && error.contains("it == 4")),
        "the qualified declaration must refuse 8 using its own predicate: {errors:?}"
    );
}

const MIXED_SUM: &str = r#"
public type Cell<N: meta USize> is { public value: Int };
public type Envelope<N: meta USize, T> is | Data(Cell<N>, T);
"#;

#[test]
fn mixed_sum_substitution_keeps_value_and_type_slots() {
    let source = r#"
mount demo.mixed.{Cell, Envelope};
fn take_cell(value: Cell<4>) {}
fn take_text(value: Text) {}
fn inspect(value: Envelope<4, Text>) {
    match value { Envelope.Data(cell, payload) => { take_cell(cell); take_text(payload); } };
}
"#;
    let errors = check(source, &[("demo.mixed", MIXED_SUM)]);
    assert!(errors.is_empty(), "{errors:?}");
    let wrong = source.replace("take_cell(value: Cell<4>)", "take_cell(value: Cell<8>)");
    let errors = check(&wrong, &[("demo.mixed", MIXED_SUM)]);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("exact value index")),
        "{errors:?}"
    );
}

#[test]
fn mixed_sum_constructor_checks_every_slot() {
    let source = r#"
mount demo.mixed.{Cell, Envelope};
fn pack(cell: Cell<4>) -> Envelope<4, Text> { Envelope.Data(cell, "value") }
"#;
    let errors = check(source, &[("demo.mixed", MIXED_SUM)]);
    assert!(errors.is_empty(), "{errors:?}");
    let wrong = source.replace("pack(cell: Cell<4>)", "pack(cell: Cell<8>)");
    let errors = check(&wrong, &[("demo.mixed", MIXED_SUM)]);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("exact value index")),
        "{errors:?}"
    );
}

#[test]
fn contextual_meta_literals_must_fit_the_declared_integer_sort() {
    for (carrier, value, valid) in [
        ("UInt8", "255", true),
        ("UInt8", "256", false),
        ("UInt64", "18446744073709551615", true),
        ("UInt64", "18446744073709551616", false),
        ("Int8", "127", true),
        ("Int8", "128", false),
        ("UInt8", "true", false),
        ("UInt8", "-1", false),
    ] {
        let source = format!(
            "type Indexed<N: meta {carrier}> is | Item(Int); fn accept(value: Indexed<{value}>) {{}}"
        );
        let errors = check(&source, &[]);
        assert_eq!(errors.is_empty(), valid, "{source}: {errors:?}");
    }
}

#[test]
fn a_symbolic_meta_argument_cannot_change_scalar_sort() {
    let source = r#"
type Indexed<N: meta USize> is | Item(Int);
fn refused<N: meta Int>(value: Indexed<N>) {}
"#;
    let errors = check(source, &[]);
    assert!(
        errors.iter().any(|error| error.contains("wrong kind")),
        "{errors:?}"
    );
}

#[test]
fn an_unresolved_payload_type_cannot_hide_a_wrong_value_index() {
    let source = r#"
mount demo.mixed.{Cell, Envelope};
fn pack<T>(cell: Cell<4>, value: T) -> Envelope<4, T> { Envelope.Data(cell, value) }
"#;
    assert!(check(source, &[("demo.mixed", MIXED_SUM)]).is_empty());
    let wrong = source.replace("pack<T>(cell: Cell<4>", "pack<T>(cell: Cell<8>");
    let errors = check(&wrong, &[("demo.mixed", MIXED_SUM)]);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("exact value index")),
        "{errors:?}"
    );
}
