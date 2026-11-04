//! Generic value identities and obligations survive actual release metadata and execution.
use verum_ast::{ItemKind, Module};
use verum_common::{List, Shared, Text};
use verum_compiler::archive_metadata::archive_to_core_metadata;
use verum_fast_parser::Parser;
use verum_types::{TypeChecker, core_metadata::CoreMetadata};
use verum_vbc::{
    archive::{ArchiveBuilder, read_archive, write_archive},
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};

const OWNER: &str = "core.value_calls";
const SOURCE: &str = r#"
module core.value_calls;
public type Cell<N: meta Int> is { public value: () };
public fn id<N: meta Int>(cell: Cell<N>) -> Cell<N> { cell }
public fn bounded<N: meta Int{>=0}>(cell: Cell<N>) -> Cell<N> { cell }
public fn mixed<A, N: meta Int, B>(first: A, cell: Cell<N>, last: B) -> Cell<N> { cell }
"#;

fn check(checker: &mut TypeChecker, ast: &Module) -> List<Text> {
    let mut errors: List<Text> = checker
        .prepare_module_declarations(ast)
        .into_iter()
        .map(|error| format!("{error:?}").into())
        .collect();
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| format!("{error:?}").into()),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.is_error())
            .map(|diagnostic| format!("{diagnostic:?}").into()),
    );
    errors
}

fn checked_source(source: &str, owner: &str) -> Module {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path(owner);
    let errors = check(&mut checker, &ast);
    assert!(errors.is_empty(), "source checking: {errors:?}");
    ast
}

fn metadata(interface: bool) -> CoreMetadata {
    let ast = checked_source(SOURCE, OWNER);
    let mut producer = VbcCodegen::with_config(CodegenConfig::new(OWNER));
    let source = if interface {
        producer.collect_declaration_interface(&[&ast])
    } else {
        producer.compile_module(&ast)
    }
    .expect("ordinary source/interface producer");
    let wire = deserialize_module(&serialize_module(&source).unwrap()).unwrap();
    let mut builder = ArchiveBuilder::new();
    builder.add_module(OWNER, &wire, &[]).unwrap();
    let mut bytes = List::new();
    write_archive(&builder.finish(), &mut bytes).unwrap();
    let archive = read_archive(std::io::Cursor::new(bytes.as_slice())).unwrap();
    let metadata = archive_to_core_metadata(&archive).expect("real archive publication");
    verum_types::core_metadata::decode_metadata(
        &verum_types::core_metadata::encode_metadata(&metadata).unwrap(),
    )
    .unwrap()
}

fn consumer_errors(metadata: &CoreMetadata, tail: &str, eager: bool) -> List<Text> {
    let metadata = Shared::new(metadata.clone()).into_arc();
    let mut checker = if eager {
        TypeChecker::new_with_core_eager(metadata)
    } else {
        TypeChecker::new_with_core(metadata)
    };
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    let mut registry = verum_modules::ModuleRegistry::new();
    registry.register(verum_modules::ModuleInfo::new(
        verum_modules::ModuleId::new(0),
        verum_modules::ModulePath::from_str(OWNER),
        Parser::new("").parse_module().unwrap(),
        verum_ast::FileId::new(0),
        Text::new(),
    ));
    checker.set_module_registry_direct(registry);
    let ast = Parser::new(&format!("mount {OWNER}.{{Cell,id,bounded,mixed}}; {tail}"))
        .parse_module()
        .expect("importing consumer grammar");
    checker.register_stdlib_types_for_module(&ast);
    check(&mut checker, &ast)
}

fn cases(consumer: &str, reason: Option<fn(&str) -> bool>) {
    for interface in [false, true] {
        let metadata = metadata(interface);
        for eager in [false, true] {
            let errors = consumer_errors(&metadata, consumer, eager);
            if let Some(reason) = reason {
                assert!(
                    errors.iter().any(|error| reason(error)),
                    "missing causal refusal, interface={interface} eager={eager} {consumer}: {errors:?}"
                );
            } else {
                assert!(
                    errors.is_empty(),
                    "interface={interface} eager={eager} {consumer}: {errors:?}"
                );
            }
        }
    }
}
fn value_mismatch(error: &str) -> bool {
    error.contains("Mismatch") || error.contains("exact value index")
}
fn domain_failure(error: &str) -> bool {
    error.contains("domain")
        || error.contains("MetaRefinementViolation")
        || error.contains("Mismatch")
}

#[test]
fn function_value_telescope_survives_wire_archive_and_sidecar() {
    let ast = Parser::new(SOURCE).parse_module().unwrap();
    for interface in [false, true] {
        let metadata = metadata(interface);
        for item in &ast.items {
            let ItemKind::Function(declaration) = &item.kind else {
                continue;
            };
            let exported = metadata
                .functions
                .get(&Text::from(format!("{OWNER}.{}", declaration.name.name)))
                .expect("exact declaring owner");
            let signature = exported
                .source_signature
                .as_ref()
                .expect("generic value signature carrier");
            assert_eq!(signature.generics, declaration.generics);
            assert!(signature.body.is_none());
        }
    }
}

#[test]
fn imported_explicit_and_inferred_value_arguments_succeed() {
    for body in [
        "id<7>(cell)",
        "id(cell)",
        "bounded<7>(cell)",
        "bounded(cell)",
        "mixed<Int,7,Bool>(1,cell,true)",
    ] {
        cases(
            &format!("fn consume(cell: Cell<7>) -> Cell<7> {{ {body} }}"),
            None,
        );
    }
}

#[test]
fn imported_calls_keep_distinct_inference_instances() {
    cases(
        "fn consume(one: Cell<1>, two: Cell<2>) -> Cell<2> { let first = id(one); id(two) }",
        None,
    );
}

#[test]
fn imported_mixed_slots_cannot_change_the_value_index() {
    for body in ["id<2>(cell)", "mixed<Int,2,Bool>(1,cell,true)"] {
        cases(
            &format!("fn consume(cell: Cell<1>) -> Cell<2> {{ {body} }}"),
            Some(value_mismatch),
        );
    }
}

#[test]
fn imported_function_cannot_change_a_rigid_caller_index() {
    cases(
        "fn consume<N: meta Int, M: meta Int>(cell: Cell<N>) -> Cell<M> { id(cell) }",
        Some(value_mismatch),
    );
}

#[test]
fn imported_parameter_domain_rejects_explicit_and_inferred_negative_values() {
    for body in ["bounded<-1>(cell)", "bounded(cell)"] {
        cases(
            &format!("fn consume(cell: Cell<-1>) -> Cell<-1> {{ {body} }}"),
            Some(domain_failure),
        );
    }
}

#[test]
fn imported_parameter_domain_preserves_symbolic_entailment() {
    cases(
        "fn consume<M: meta Int{>=7}>(cell: Cell<M>) -> Cell<M> { bounded(cell) }",
        None,
    );
    cases(
        "fn consume<M: meta Int>(cell: Cell<M>) -> Cell<M> { bounded(cell) }",
        Some(domain_failure),
    );
}

#[test]
fn imported_value_parameter_rejects_a_different_scalar_sort() {
    cases(
        "fn consume(cell: Cell<1>) -> Cell<1> { id<true>(cell) }",
        Some(|error| {
            error.contains("wrong kind") || error.contains("sort") || error.contains("Mismatch")
        }),
    );
}

fn execute_size_call(call: &str) {
    let source = format!(
        r#"module value_execution;
        type Cell<N: meta Int> is {{ value: () }};
        fn size<N: meta Int>(cell: Cell<N>) -> Int {{ N }}
        fn probe() -> Int {{ let cell: Cell<7> = Cell {{ value: () }}; {call} }}
    "#
    );
    let ast = checked_source(&source, "value_execution");
    let module = VbcCodegen::with_config(CodegenConfig::new("value_execution"))
        .compile_module(&ast)
        .expect("checked source compiles to VBC");
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, wire] {
        let entry = module
            .find_function_by_name("value_execution.probe")
            .expect("actual compiled entry");
        let value = Interpreter::new(Shared::new(module).into_arc())
            .execute_function(entry)
            .expect("generic value execution succeeds");
        assert_eq!(
            value.try_as_i64(),
            Some(7),
            "the body must observe its instantiated value parameter"
        );
    }
}

#[test]
fn explicit_value_parameter_executes_through_source_and_wire() {
    execute_size_call("size<7>(cell)");
}

#[test]
fn inferred_value_parameter_executes_through_source_and_wire() {
    execute_size_call("size(cell)");
}

#[test]
fn imported_function_value_binders_do_not_capture_the_callers_same_spelled_parameter() {
    cases("fn consume<N: meta Int>(cell: Cell<N>) -> Cell<N> { id(cell) }", None);
    cases("fn consume<N: meta Int>(cell: Cell<N>) -> Cell<7> { id<7>(cell) }", Some(value_mismatch));
    cases("fn consume<N: meta Int, M: meta Int>(cell: Cell<N>) -> Cell<M> { id<N>(cell) }", Some(value_mismatch));
    cases("fn consume<N: meta Int>(cell: Cell<7>) -> Cell<7> { id<7>(cell) }", None);
}
