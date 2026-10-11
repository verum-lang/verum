//! T1733: each imported sum instance carries its own success and error payload.
use std::sync::Arc;
use verum_ast::{FileId, ItemKind};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::{TypeChecker, core_metadata::CoreMetadata};

const CARRIER: &str = "public type Outcome<T, E> is | Ok(T) | Err(E);";
const PROVIDER: &str = r#"
mount demo.carrier.Outcome;
public type First is { public count: Int };
public type Second is { public text: Text };
public type Third is { public flag: Bool };
public fn first() -> Outcome<First, Int> { Outcome.Ok(First { count: 3 }) }
public fn second() -> Outcome<Second, Text> { Outcome.Ok(Second { text: "second" }) }
public fn third() -> Outcome<Third, Bool> { Outcome.Ok(Third { flag: true }) }
"#;

fn check_with_checker(
    consumer: &str,
    modules: &[(&str, &str)],
    metadata: Option<CoreMetadata>,
    eager: bool,
    warm_archive_aliases: bool,
) -> (TypeChecker, List<Text>) {
    let mut checker = match metadata {
        Some(metadata) if eager => TypeChecker::new_with_core_eager(Arc::new(metadata)),
        Some(metadata) => TypeChecker::new_with_core(Arc::new(metadata)),
        None => TypeChecker::new(),
    };
    checker.register_primitives();
    if warm_archive_aliases {
        let mut pending = Default::default();
        for name in ["Result", "core.base.result.Result"] {
            checker.ensure_stdlib_type_loaded(&Text::from(name), &mut pending);
        }
    }
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
            .expect("source grammar");
        let id = ModuleId::new(index as u32);
        let path = ModulePath::from_str(owner);
        let mut module = ModuleInfo::new(id, path.clone(), ast.clone(), file_id, (*source).into());
        module.exports = extract_exports_from_module(&ast, id, &path).expect("source exports");
        registry.register(module);
    }
    checker.set_module_registry_direct(registry.clone());
    let ast = FastParser::new()
        .parse_module_str(consumer, FileId::new(modules.len() as u32))
        .expect("consumer grammar");
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
            ItemKind::Impl(decl) => checker.register_impl_block(decl),
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
    (checker, errors)
}

fn check(consumer: &str, modules: &[(&str, &str)]) -> List<Text> {
    check_with_checker(consumer, modules, None, false, false).1
}

fn source(order: &[usize], separate_functions: bool, bad: bool) -> Text {
    let checks = [
        "match first() { Outcome.Ok(value) => take_first(value), Outcome.Err(error) => take_int(error) };",
        "match second() { Outcome.Ok(value) => take_second(value), Outcome.Err(error) => take_text(error) };",
        "match third() { Outcome.Ok(value) => take_third(value), Outcome.Err(error) => take_bool(error) };",
    ];
    let mut source = Text::from(
        r#"
mount demo.carrier.Outcome;
mount demo.provider.{First, Second, Third, first, second, third};
fn take_first(value: First) {}
fn take_second(value: Second) {}
fn take_third(value: Third) {}
fn take_int(value: Int) {}
fn take_text(value: Text) {}
fn take_bool(value: Bool) {}
"#,
    );
    if !separate_functions {
        source.push_str("fn probe() {");
    }
    for (index, selected) in order.iter().enumerate() {
        if separate_functions {
            source.push_str(&format!("fn probe{index}() {{"));
        }
        let mut check = checks[*selected].to_owned();
        if bad && *selected == 1 {
            check = check.replace("take_second(value)", "take_first(value)");
        }
        source.push_str(&check);
        if separate_functions {
            source.push_str("}");
        }
    }
    if !separate_functions {
        source.push_str("}");
    }
    source
}

#[test]
fn imported_sum_instances_keep_distinct_payloads_within_one_caller() {
    for order in [[0, 1, 2], [2, 1, 0]] {
        let errors = check(
            &source(&order, false, false),
            &[("demo.carrier", CARRIER), ("demo.provider", PROVIDER)],
        );
        assert!(errors.is_empty(), "order={order:?}: {errors:?}");
    }
}

#[test]
fn imported_sum_instances_keep_distinct_payloads_across_callers() {
    for order in [[0, 1, 2], [2, 1, 0]] {
        let errors = check(
            &source(&order, true, false),
            &[("demo.carrier", CARRIER), ("demo.provider", PROVIDER)],
        );
        assert!(errors.is_empty(), "order={order:?}: {errors:?}");
    }
}

#[test]
fn wrong_payload_owner_is_refused_independently_of_previous_matches() {
    for separate in [false, true] {
        let errors = check(
            &source(&[0, 1, 2], separate, true),
            &[("demo.carrier", CARRIER), ("demo.provider", PROVIDER)],
        );
        assert!(!errors.is_empty(), "distinct payloads must not unify");
        assert!(
            errors.iter().any(|error| error.contains("Mismatch")
                && error.contains("First")
                && error.contains("Second")),
            "{errors:?}"
        );
    }
}

// These two exact descriptor rows were extracted from the ordinary stage22
// metadata whose SHA256 is cba1131412a3a460cab4f85ae7f0f05777492de0f9f3190fc4601beef76d48c5.
// Other metadata collections are empty; this is a consumer isolation control,
// not a replacement for the real project runtime replay.
fn captured_metadata() -> CoreMetadata {
    serde_json::from_str(include_str!("fixtures/stage22_result_metadata.json")).unwrap()
}

fn metadata_errors(order: &[usize], separate: bool, bad: bool, eager: bool) -> List<Text> {
    let source = source(order, separate, bad)
        .replace(
            "mount demo.carrier.Outcome;",
            "mount core.base.result.Result;",
        )
        .replace("Outcome", "Result");
    let provider = PROVIDER
        .replace(
            "mount demo.carrier.Outcome;",
            "mount core.base.result.Result;",
        )
        .replace("Outcome", "Result");
    check_with_checker(
        &source,
        &[("core.base.result", ""), ("demo.provider", &provider)],
        Some(captured_metadata()),
        eager,
        false,
    )
    .1
}

#[test]
fn captured_archive_sum_keeps_distinct_payloads_within_one_caller() {
    for eager in [false, true] {
        for order in [[0, 1, 2], [2, 1, 0]] {
            let errors = metadata_errors(&order, false, false, eager);
            assert!(
                errors.is_empty(),
                "eager={eager}, order={order:?}: {errors:?}"
            );
        }
    }
}

#[test]
fn captured_archive_sum_keeps_distinct_payloads_across_callers() {
    for eager in [false, true] {
        for order in [[0, 1, 2], [2, 1, 0]] {
            let errors = metadata_errors(&order, true, false, eager);
            assert!(
                errors.is_empty(),
                "eager={eager}, order={order:?}: {errors:?}"
            );
        }
    }
}

#[test]
fn captured_archive_sum_refuses_wrong_payload_owner() {
    for eager in [false, true] {
        let errors = metadata_errors(&[0, 1, 2], false, true, eager);
        assert!(
            !errors.is_empty(),
            "distinct metadata payloads must not unify"
        );
        assert!(
            errors.iter().any(|error| error.contains("Mismatch")
                && error.contains("First")
                && error.contains("Second")),
            "{errors:?}"
        );
    }
}

fn dual_key_errors(qualified: bool, eager: bool, order: &[usize], bad: bool) -> List<Text> {
    let consumer = source(order, false, bad)
        .replace("mount demo.carrier.Outcome;", "")
        .replace("Outcome", "Result");
    let mut provider = PROVIDER
        .replace("mount demo.carrier.Outcome;", "")
        .replace("Outcome", "Result");
    if qualified {
        provider = provider.replace("-> Result<", "-> core.base.result.Result<");
    }
    check_with_checker(
        &consumer,
        &[("core.base.result", ""), ("demo.provider", &provider)],
        Some(captured_metadata()),
        eager,
        true,
    )
    .1
}

#[test]
fn ambient_archive_payloads_keep_their_own_loaded_parameter_slots() {
    for eager in [false, true] {
        for order in [[0, 1, 2], [2, 1, 0]] {
            let errors = dual_key_errors(false, eager, &order, false);
            assert!(
                errors.is_empty(),
                "eager={eager}, order={order:?}: {errors:?}"
            );
        }
    }
}

#[test]
fn qualified_archive_payloads_keep_their_own_loaded_parameter_slots() {
    for eager in [false, true] {
        for order in [[0, 1, 2], [2, 1, 0]] {
            let errors = dual_key_errors(true, eager, &order, false);
            assert!(
                errors.is_empty(),
                "eager={eager}, order={order:?}: {errors:?}"
            );
        }
    }
}
