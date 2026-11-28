//! Checked selections must reach the same nested bodies that were admitted.
use verum_ast::{Expr, ExprKind, Module, Span, Visitor, expr::ResolvedCallTarget};
use verum_common::{List, Map, Maybe, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

#[derive(Default)]
struct Calls(List<(Span, Maybe<ResolvedCallTarget>)>);
impl Visitor for Calls {
    fn visit_expr(&mut self, expr: &Expr) {
        if matches!(expr.kind, ExprKind::MethodCall { .. } | ExprKind::Path(_)) {
            self.0.push((expr.span, expr.resolved_call_target.clone()));
        }
    }
}
fn selections(module: &Module) -> Calls {
    let mut calls = Calls::default();
    for item in &module.items {
        calls = verum_ast::visitor::traverse_item_iteratively(calls, item);
    }
    calls
}
fn checker_for(ast: &Module) -> TypeChecker {
    use verum_ast::ItemKind;
    use verum_modules::{ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
    // Keep complete inline modules as source-owned entries, as compile_core does.
    let mut registry = ModuleRegistry::new();
    let mut pending = List::from_iter([(Text::from("fixture"), ast.clone())]);
    while let Some((owner, module)) = pending.pop() {
        for item in &module.items {
            if let ItemKind::Module(declaration) = &item.kind
                && let Some(items) = &declaration.items
            {
                pending.push((
                    format!("{owner}.{}", declaration.name.name).into(),
                    Module {
                        items: items.clone(),
                        attributes: item.attributes.clone(),
                        file_id: module.file_id,
                        span: item.span,
                    },
                ));
            }
        }
        let id = registry.allocate_id();
        let path = ModulePath::from_str(&owner);
        let exports = extract_exports_from_module(&module, id, &path).unwrap();
        let file = module.file_id;
        let mut info = ModuleInfo::new(id, path, module, file, Text::new());
        info.exports = exports;
        registry.register(info);
    }
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_builtins();
    checker.set_current_cog("fixture");
    checker.set_current_module_path("fixture");
    checker.set_module_registry_direct(registry);
    checker
}
fn check(source: &str) -> (Module, TypeChecker) {
    let ast = Parser::new(source).parse_module().expect("actual grammar");
    let mut checker = checker_for(&ast);
    let mut errors = checker.prepare_module_declarations(&ast);
    errors.extend(
        ast.items
            .iter()
            .filter_map(|item| checker.check_item(item).err()),
    );
    errors.extend(checker.take_deferred_errors());
    assert!(errors.is_empty(), "{source}\n{errors:?}");
    assert!(
        checker.diagnostic_sources().is_empty(),
        "{:?}",
        checker.diagnostic_sources()
    );
    (ast, checker)
}
fn assert_committed(source: &str, copies: usize, static_calls: bool) {
    let (mut ast, mut checker) = check(source);
    let mut free_route = ast.clone();
    checker.commit_resolved_call_targets(&mut ast);
    let first = selections(&ast).0;
    checker.commit_resolved_call_targets(&mut ast);
    assert_eq!(selections(&ast).0, first, "commit must be idempotent");
    let expected = checker.take_resolved_call_targets();
    assert_eq!(
        expected
            .values()
            .filter(|target| matches!(target, ResolvedCallTarget::ValueCopy))
            .count(),
        copies
    );
    if static_calls {
        assert!(
            expected
                .values()
                .any(|target| matches!(target, ResolvedCallTarget::StaticCall { .. })),
            "no selected static call: {expected:?}"
        );
    }
    let derefs = checker.take_deref_adjustments();
    verum_types::apply_resolved_call_targets(&mut free_route, &expected, &derefs);
    assert_eq!(
        selections(&free_route).0,
        first,
        "both public commit routes agree"
    );
    let actual: Map<_, _> = first.into_iter().collect();
    for (span, target) in &expected {
        assert_eq!(
            actual.get(span),
            Some(&Some(target.clone())),
            "lost checked selection at {span:?}"
        );
    }
}

#[test]
fn nested_namespaces_keep_copy_and_declared_method_selections() {
    assert_committed(
        r#"
        module outer { module inner {
            type Leaf is { value:Int };
            implement Leaf { fn read(&self)->Int {self.value} }
            fn duplicate(source:&Leaf)->Leaf {source.clone()}
            fn inspect(source:&Leaf)->Int {source.read()}
            type Choice is Empty | Full(Int);
            fn choose()->Choice {Choice.Full(73)}
        }}
    "#,
        1,
        true,
    );

}

#[test]
fn generic_protocol_defaults_keep_selected_copy() {
    assert_committed(
        r#"
        type Leaf is { value:Int };
        type Factory<A> is protocol {
            fn duplicate(&self, source:&Leaf)->Leaf {source.clone()}
            fn marker(&self, value:A)->A;
        };
    "#,
        1,
        false,
    );
}

#[test]
fn nested_default_and_explicit_override_keep_distinct_selections() {
    assert_committed(
        r#"
        module outer {
            type Leaf is {value:Int};
            implement Leaf {fn next(&self)->Leaf {Leaf {value:self.value+1}}}
            type Factory<A> is protocol {
                fn duplicate(&self, source:&Leaf)->Leaf {source.clone()}
                fn marker(&self, value:A)->A;
            };
            type Maker is {seed:Int};
            implement Factory<Int> for Maker {
                fn duplicate(&self, source:&Leaf)->Leaf {source.next()}
                fn marker(&self, value:Int)->Int {value}
            }
        }
    "#,
        1,
        true,
    );
}

#[test]
fn nested_default_does_not_authorize_invalid_copy() {
    for declaration in [
        "type affine Leaf is {value:Int};",
        "type Leaf is {value:&mut Int};",
        "type Leaf is {value:&unsafe Int};",
        "type Leaf is {value:fn()->Int};",
    ] {
        let source = format!(
            "module outer {{{declaration} type Factory<A> is protocol {{fn duplicate(&self, source:&Leaf)->Leaf {{source.clone()}}}};}}"
        );
        let ast = Parser::new(&source).parse_module().unwrap();
        let mut checker = checker_for(&ast);
        let mut errors = checker.prepare_module_declarations(&ast);
        errors.extend(
            ast.items
                .iter()
                .filter_map(|item| checker.check_item(item).err()),
        );
        errors.extend(checker.take_deferred_errors());
        assert!(
            errors
                .iter()
                .any(|error| error.to_diagnostic().code() == Some("E400")),
            "{source}: {errors:?}"
        );
    }
}

#[test]
fn a_checked_factory_binding_retains_only_its_closed_nominal_arguments() {
    #[derive(Default)]
    struct Bindings(Map<Text, Maybe<Text>>);
    impl Visitor for Bindings {
        fn visit_stmt(&mut self, statement: &verum_ast::Stmt) {
            if let verum_ast::StmtKind::Let { pattern, ty, .. } = &statement.kind
                && let verum_ast::PatternKind::Ident { name, .. } = &pattern.kind
            {
                self.0.insert(name.name.clone(), ty.as_ref().map(verum_ast::pretty::format_type));
            }
        }
    }
    let source = r#"
        module records {
            public type Batch<T> is { value: T };
            fn batch<T>(x: T) -> Batch<T> { Batch {value: x} }
            fn concrete() -> Batch<Int> { let inferred = batch(17); inferred }
            fn generic<T>(x: T) -> Batch<T> { let open = batch(x); open }
        }
    "#;
    let (original, mut checker) = check(source);
    let mut direct = original.clone();
    checker.commit_resolved_call_targets(&mut direct);
    let mut transferred = original;
    let registry = checker.take_type_registry();
    registry.apply_checked_source_facts(
        &mut transferred,
        &checker.take_resolved_call_targets(),
        &checker.take_deref_adjustments(),
    );
    for module in [direct, transferred] {
        let mut bindings = Bindings::default();
        for item in &module.items {
            bindings = verum_ast::visitor::traverse_item_iteratively(bindings, item);
        }
        assert_eq!(bindings.0.get(&Text::from("inferred")).and_then(Option::as_deref), Some("fixture.records.Batch<Int>"));
        assert_eq!(bindings.0.get(&Text::from("open")), Some(&None));
    }
}
