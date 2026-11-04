//! Consumer-only audit: real parsed declaration carriers; no VBC/archive producer claim.
use std::sync::Arc;
use verum_ast::{ItemKind, Module};
use verum_common::{List, ResourceDiscipline, Text};
use verum_fast_parser::Parser;
use verum_types::{TypeChecker, core_metadata::*};
const OWNER: &str = "scope.provider";
fn parse(s: &str) -> Module {
    Parser::new(s).parse_module().expect("grammar")
}
fn check(checker: &mut TypeChecker, ast: &Module) -> List<Text> {
    let mut errors: List<Text> = checker
        .prepare_module_declarations(ast)
        .into_iter()
        .map(|e| format!("{e:?}").into())
        .collect();
    for item in &ast.items {
        if let Err(e) = checker.check_item(item) {
            errors.push(format!("{e:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| format!("{e:?}").into()),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| format!("{d:?}").into()),
    );
    errors
}
fn source_errors(s: &str) -> List<Text> {
    let mut c = TypeChecker::new();
    c.register_primitives();
    c.set_current_module_path(OWNER);
    check(&mut c, &parse(s))
}
fn metadata(function: &str) -> CoreMetadata {
    let mut m = CoreMetadata {
        version: CORE_METADATA_VERSION,
        ..CoreMetadata::default()
    };
    let ast = parse("public type Cell<N: meta Int> is { public value: () };");
    let ItemKind::Type(td) = &ast.items[0].kind else {
        panic!("type")
    };
    let td = TypeDescriptor {
        source_declaration: verum_ast::source_signature::source_type_declaration(td).unwrap(),
        name: "Cell".into(),
        module_path: OWNER.into(),
        origin_module_path: None,
        generic_params: List::new(),
        kind: TypeDescriptorKind::Record {
            fields: List::new(),
        },
        size: None,
        alignment: None,
        methods: List::new(),
        implements: List::new(),
        decl_span: None,
        is_public: true,
        is_transparent_wrapper: false,
        resource_discipline: ResourceDiscipline::Unrestricted,
    };
    m.types.insert(format!("{OWNER}.Cell").into(), td);
    let ast = parse(function);
    let ItemKind::Function(fd) = &ast.items[0].kind else {
        panic!("function")
    };
    let source = verum_ast::source_signature::source_signature(fd, false)
        .unwrap()
        .expect("value header carrier");
    let fd = FunctionDescriptor {
        name: source.name.name.clone(),
        module_path: OWNER.into(),
        origin_module_path: None,
        source_signature: Some(source),
        generic_params: List::new(),
        params: List::from_single(ParamDescriptor {
            name: "cell".into(),
            ty: "__opaque_src".into(),
            declared_ty: "__opaque_src".into(),
            has_default: false,
            default_literal: None,
        }),
        return_type: "__opaque_src".into(),
        contexts: List::new(),
        is_async: false,
        is_unsafe: false,
        intrinsic_id: None,
        parent_type: None,
        impl_generic_names: List::new(),
        is_const: false,
        decl_span: None,
        is_public: true,
        explicit_type_param_ids: None,
    };
    m.functions
        .insert(format!("{OWNER}.{}", fd.name).into(), fd);
    m
}
fn consumer(function: &str, tail: &str, eager: bool) -> List<Text> {
    let m = Arc::new(metadata(function));
    let mut c = if eager {
        TypeChecker::new_with_core_eager(m)
    } else {
        TypeChecker::new_with_core(m)
    };
    c.register_primitives();
    c.set_current_module_path("scope.consumer");
    c.set_current_cog("scope");
    check(&mut c, &parse(tail))
}
const ID: &str = "public fn id<N: meta Int>(cell: Cell<N>) -> Cell<N> { cell }";
#[test]
fn positive_declared_owner_replayed_independently() {
    assert!(
        source_errors(&format!(
            "public type Cell<N: meta Int> is {{ public value: () }}; {ID}"
        ))
        .is_empty()
    );
    for eager in [false, true] {
        for tail in [
            "fn caller<N: meta Int>(cell: scope.provider.Cell<N>) -> scope.provider.Cell<N> { scope.provider.id(cell) }",
            "fn caller<N: meta Int>(cell: scope.provider.Cell<7>) -> scope.provider.Cell<7> { scope.provider.id<7>(cell) }",
        ] {
            let es = consumer(ID, tail, eager);
            assert!(es.is_empty(), "eager={eager}: {es:?}");
        }
    }
}
#[test]
fn negative_import_cannot_identify_distinct_caller_parameters() {
    for eager in [false, true] {
        let es = consumer(
            ID,
            "fn caller<N: meta Int,M: meta Int>(cell: scope.provider.Cell<N>) -> scope.provider.Cell<M> { scope.provider.id<N>(cell) }",
            eager,
        );
        assert!(
            es.iter()
                .any(|e| e.contains("Mismatch") || e.contains("exact value")),
            "eager={eager}: {es:?}"
        );
    }
}
#[test]
fn negative_import_cannot_capture_undeclared_value_binder() {
    let bad = "public fn leaked<K: meta Int>(cell: Cell<N>) -> Cell<N> { cell }";
    let own = source_errors(&format!(
        "public type Cell<N: meta Int> is {{ public value: () }}; {bad}"
    ));
    assert!(
        own.iter().any(|e| e.contains("N")
            && (e.contains("Undefined")
                || e.contains("NotFound")
                || (e.contains("E407") && e.contains("wrong kind")))),
        "invalid provider must refuse undeclared N: {own:?}"
    );
    for eager in [false, true] {
        let es = consumer(
            bad,
            "fn caller<N: meta Int>(cell: scope.provider.Cell<N>) -> scope.provider.Cell<N> { scope.provider.leaked<0>(cell) }",
            eager,
        );
        assert!(
            es.iter()
                .any(|e| e.contains("TypeNotFound") && e.contains("N")),
            "consumer must refuse undeclared N: eager={eager}: {es:?}"
        );
    }
}
#[test]
fn negative_import_cannot_borrow_ambient_refinement_constant() {
    let bad = "public fn bounded<M: meta Int{it >= N}>(cell: Cell<M>) -> Cell<M> { cell }";
    for eager in [false, true] {
        let es = consumer(
            bad,
            "fn caller<N: meta Int>(cell: scope.provider.Cell<7>) -> scope.provider.Cell<7> { scope.provider.bounded<7>(cell) }",
            eager,
        );
        assert!(
            es.iter().any(|e| e.contains("domain")
                || e.contains("Undefined")
                || e.contains("Unresolved")),
            "ambient predicate eager={eager}: {es:?}"
        );
    }
}
#[test]
fn negative_owned_function_value_cannot_become_foreign_constant() {
    for eager in [false, true] {
        let es = consumer(
            ID,
            "fn caller<N: meta Int>(cell: scope.provider.Cell<N>) -> scope.provider.Cell<7> { scope.provider.id<7>(cell) }",
            eager,
        );
        assert!(
            es.iter()
                .any(|e| e.contains("Mismatch") || e.contains("exact value")),
            "eager={eager}: {es:?}"
        );
    }
}

#[test]
fn imported_erased_lifetime_preserves_the_following_type_slot() {
    let source = "public fn id<'a,T>(cell:T)->T { cell }";
    assert!(
        source_errors(&format!(
            "{source} fn probe()->Int {{ id<'static,Int>(7) }}"
        ))
        .is_empty()
    );
    for eager in [false, true] {
        let errors = consumer(
            source,
            "fn probe()->Int { scope.provider.id<'static,Int>(7) }",
            eager,
        );
        assert!(errors.is_empty(), "eager={eager}: {errors:?}");
        let errors = consumer(
            source,
            "fn probe()->Int { scope.provider.id<Int,Int>(7) }",
            eager,
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("lifetime parameter")),
            "eager={eager}: {errors:?}"
        );
    }
}
