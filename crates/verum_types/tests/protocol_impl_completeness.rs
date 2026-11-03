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

const ALPHA: &str = "public type Demand is protocol { fn keep(&self, value: Int) -> Int; };";
const BETA: &str = "public type Demand is protocol { fn other(&self, value: Int) -> Int; };";

#[test]
fn extra_method_does_not_hide_missing_required_method() {
    let result = errors(
        "type Demand is protocol { fn keep(&self, value: Int) -> Int; }; type Receiver is { value: Int }; implement Demand for Receiver { fn other(&self, value: Int) -> Int { value } }",
        &[],
    );
    assert!(
        result
            .iter()
            .any(|e| e.contains("E405") && e.contains("keep")),
        "missing required method must be diagnosed: {result:?}"
    );
}

#[test]
fn a_complete_implementation_may_have_an_additional_method() {
    let result = errors(
        "type Demand is protocol { fn keep(&self, value: Int) -> Int; }; type Receiver is { value: Int }; implement Demand for Receiver { fn keep(&self, value: Int) -> Int { value } fn extra(&self, value: Int) -> Int { value } }",
        &[],
    );
    assert!(result.is_empty(), "{result:?}");
}

#[test]
fn default_methods_do_not_become_required_during_owned_replay() {
    let result = errors(
        "type Demand is protocol { fn keep(&self, value: Int) -> Int { value } }; type Receiver is { value: Int }; implement Demand for Receiver {} fn read(receiver: &Receiver) -> Int { receiver.keep(73) }",
        &[],
    );
    assert!(result.is_empty(), "{result:?}");
}

#[test]
fn a_root_declaration_does_not_require_a_module_registry() {
    assert_standalone_implementation("cog");
}

#[test]
fn a_scoped_declaration_does_not_require_a_module_registry() {
    assert_standalone_implementation("demo.main");
}

fn assert_standalone_implementation(owner: &str) {
    let ast = FastParser::new()
        .parse_module_str(
            "type Demand is protocol { fn keep(&self, value: Int) -> Int; }; type Receiver is { value: Int }; implement Demand for Receiver { fn keep(&self, value: Int) -> Int { value } }",
            FileId::new(0),
        )
        .unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path(owner);
    assert!(checker.prepare_module_declarations(&ast).is_empty());
    for item in &ast.items {
        checker.check_item(item).unwrap();
    }
    assert!(checker.take_deferred_errors().is_empty());
    assert!(
        checker
            .diagnostics()
            .iter()
            .all(|diagnostic| !diagnostic.is_error()),
        "{:?}",
        checker.diagnostics()
    );
    let identity = if owner == "cog" {
        "Demand".to_owned()
    } else {
        format!("{owner}.Demand")
    };
    assert!(
        checker
            .protocol_checker
            .read()
            .get_protocol(&identity.into())
            .is_some(),
        "canonical declaration must be registered"
    );
}

#[test]
fn same_leaf_protocol_aliases_select_their_own_declarations_in_both_orders() {
    for reverse in [false, true] {
        let mounts = if reverse {
            "mount demo.beta.{Demand as B}; mount demo.alpha.{Demand as A};"
        } else {
            "mount demo.alpha.{Demand as A}; mount demo.beta.{Demand as B};"
        };
        let source = format!(
            "{mounts} type Left is {{ value: Int }}; type Right is {{ value: Int }}; implement A for Left {{ fn keep(&self, value: Int) -> Int {{ value }} }} implement B for Right {{ fn other(&self, value: Int) -> Int {{ value }} }}"
        );
        let result = errors(&source, &[("demo.alpha", ALPHA), ("demo.beta", BETA)]);
        assert!(result.is_empty(), "reverse={reverse}: {result:?}");
    }
}

#[test]
fn a_same_leaf_protocol_cannot_supply_another_declarations_required_method() {
    for reverse in [false, true] {
        let mounts = if reverse {
            "mount demo.beta.{Demand as B}; mount demo.alpha.{Demand as A};"
        } else {
            "mount demo.alpha.{Demand as A}; mount demo.beta.{Demand as B};"
        };
        let source = format!(
            "{mounts} type Receiver is {{ value: Int }}; implement A for Receiver {{ fn other(&self, value: Int) -> Int {{ value }} }}"
        );
        let result = errors(&source, &[("demo.alpha", ALPHA), ("demo.beta", BETA)]);
        assert!(
            result.iter().any(|e| e.contains("E405")
                && e.contains("keep")
                && e.contains("demo.alpha.Demand")),
            "reverse={reverse}: exact declaration must refuse: {result:?}"
        );
    }
}

#[test]
fn qualified_protocol_paths_keep_both_owners() {
    let source = "type Left is { value: Int }; type Right is { value: Int }; implement demo.alpha.Demand for Left { fn keep(&self, value: Int) -> Int { value } } implement demo.beta.Demand for Right { fn other(&self, value: Int) -> Int { value } }";
    let result = errors(source, &[("demo.alpha", ALPHA), ("demo.beta", BETA)]);
    assert!(result.is_empty(), "{result:?}");
}

#[test]
fn qualified_protocol_cannot_borrow_a_leaf_declarations_requirements() {
    let source = "mount demo.beta.{Demand}; type Receiver is { value: Int }; implement demo.alpha.Demand for Receiver { fn other(&self, value: Int) -> Int { value } }";
    let result = errors(source, &[("demo.alpha", ALPHA), ("demo.beta", BETA)]);
    assert!(
        result
            .iter()
            .any(|e| e.contains("E405") && e.contains("keep") && e.contains("demo.alpha.Demand")),
        "exact declaration must refuse: {result:?}"
    );
}

fn standalone_protocol_fixture(body: &str) -> (TypeChecker, verum_ast::decl::ImplDecl) {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("demo.main");
    let source = format!(
        r#"
        type Demand is protocol {{ fn keep(&self, value: Int) -> Int; }};
        type Receiver is {{ value: Int }};
        implement demo.main.Demand for demo.main.Receiver {{
            fn keep(&self, value: Int) -> Int {{ {body} }}
        }}
    "#
    );
    let ast = FastParser::new()
        .parse_module_str(&source, FileId::new(0))
        .unwrap();
    let mut implementation = None;
    for item in &ast.items {
        match &item.kind {
            ItemKind::Type(decl) => checker.register_type_declaration(decl).unwrap(),
            ItemKind::Impl(decl) => implementation = Some(decl.clone()),
            _ => {}
        }
    }
    (checker, implementation.unwrap())
}

#[test]
fn repeated_exact_source_registration_is_idempotent() {
    let (mut checker, declaration) = standalone_protocol_fixture("value");
    checker.register_impl_block(&declaration).unwrap();
    let before = checker.protocol_checker.read().all_implementations().len();
    checker.register_impl_block(&declaration).unwrap();
    assert_eq!(
        checker.protocol_checker.read().all_implementations().len(),
        before
    );
}

#[test]
fn identical_source_impls_in_distinct_owners_do_not_share_a_receipt() {
    let (mut checker, declaration) = standalone_protocol_fixture("value");
    checker.register_impl_block(&declaration).unwrap();
    checker.set_current_module_path("demo.other");
    let error = checker.register_impl_block(&declaration).unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("Overlapping implementations")
            && message.contains("demo.main.Demand")
            && message.contains("demo.main.Receiver"),
        "{message}"
    );
    assert_eq!(error.span(), declaration.span);
}

#[test]
fn identical_impls_at_distinct_source_spans_conflict() {
    let result = errors(
        r#"
        type Demand is protocol { fn keep(&self, value: Int) -> Int; };
        type Receiver is { value: Int };
        implement Demand for Receiver { fn keep(&self, value: Int) -> Int { value } }
        implement Demand for Receiver { fn keep(&self, value: Int) -> Int { value } }
    "#,
        &[],
    );
    assert!(
        result
            .iter()
            .any(|e| e.contains("Overlapping implementations")
                && e.contains("demo.main.Demand")
                && e.contains("demo.main.Receiver")),
        "{result:?}"
    );
}

#[test]
fn a_registration_receipt_does_not_skip_method_body_checking() {
    let result = errors(
        r#"
        type Demand is protocol { fn keep(&self, value: Int) -> Int; };
        type Receiver is { value: Int };
        implement Demand for Receiver { fn keep(&self, value: Int) -> Int { true } }
    "#,
        &[],
    );
    assert!(
        result
            .iter()
            .any(|e| e.contains("Mismatch") && e.contains("Int") && e.contains("Bool")),
        "{result:?}"
    );
    assert!(
        !result
            .iter()
            .any(|e| e.contains("Overlapping implementations")),
        "{result:?}"
    );
}

#[test]
fn source_receipt_requires_exact_declaration_at_its_recorded_location() {
    let (mut checker, mut declaration) = standalone_protocol_fixture("value");
    checker.register_impl_block(&declaration).unwrap();
    declaration.is_unsafe = !declaration.is_unsafe;
    let error = checker.register_impl_block(&declaration).unwrap_err();
    assert!(
        error.to_string().contains("changed after publication"),
        "{error}"
    );
    assert_eq!(error.span(), declaration.span);
}
