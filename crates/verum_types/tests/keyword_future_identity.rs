//! Intrinsic spawned futures must not borrow a library record's nominal name.
use verum_ast::{FileId, ItemKind};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::{Type, TypeChecker};

#[test]
fn keyword_spawn_preserves_its_intrinsic_output_identity() {
    for source in ["spawn { 7 }", "spawn(async { 7 })"] {
        let expression = FastParser::new().parse_expr_str(source, FileId::new(0)).unwrap();
        let mut checker = TypeChecker::new();
        checker.register_primitives();
        let result = checker.synth_expr(&expression).unwrap();
        assert_eq!(result.ty, Type::Future { output: Box::new(Type::Int) }, "{source}");
        let resolution = checker.protocol_checker.read().resolve_future_protocol(&result.ty).unwrap();
        assert_eq!(resolution.output, Type::Int);
    }
}

#[test]
fn a_nominal_handle_does_not_acquire_the_keyword_output_by_name() {
    let source = r#"
        type JoinHandle<T> is { value: T };
        async fn wrong(handle: JoinHandle<Int>) -> Int { handle.await }
    "#;
    let ast = FastParser::new().parse_module_str(source, FileId::new(0)).unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    for item in &ast.items {
        match &item.kind {
            ItemKind::Type(decl) => checker.register_type_declaration(decl).unwrap(),
            ItemKind::Function(decl) => checker.register_function_signature(decl).unwrap(),
            _ => (),
        }
    }
    let errors: Vec<Text> = ast.items.iter().filter_map(|item| checker.check_item(item).err().map(|e| format!("{e:?}").into())).collect();
    assert!(!errors.is_empty(), "a record cannot yield its type argument without a Future implementation");
}

#[test]
fn a_future_output_equality_rejects_an_incompatible_call() {
    for (value, accepted) in [("7", true), ("\"wrong output\"", false)] {
        let source = format!(r#"
            type Future is protocol {{
                type Output;
                fn get(&self) -> Self.Output;
            }};
            type Ready<T> is {{ value: T }};
            implement<T> Future for Ready<T> {{
                type Output = T;
                fn get(&self) -> T {{ self.value }}
            }}
            type BorrowedOutput is {{ value: Int }};
            implement Future for BorrowedOutput {{
                type Output = &Int;
                fn get(&self) -> &Int {{ &self.value }}
            }}
            fn exact<T, F: Future<Output = T>>(value: &F) -> T {{ value.get() }}
            fn escaping<T, F: Future<Output = T>>(owner: F) -> T {{ owner.get() }}
            fn probe() -> Int {{ let ready = Ready {{ value: {value} }}; exact(&ready) }}
        "#);
        let ast = FastParser::new().parse_module_str(&source, FileId::new(0)).unwrap();
        let mut checker = TypeChecker::new();
        checker.register_primitives();
        for item in &ast.items {
            match &item.kind {
                ItemKind::Type(decl) => checker.register_type_declaration(decl).unwrap(),
                ItemKind::Impl(decl) => checker.register_impl_block(decl).unwrap(),
                ItemKind::Function(decl) => checker.register_function_signature(decl).unwrap(),
                _ => (),
            }
        }
        for item in &ast.items {
            if matches!(&item.kind, ItemKind::Function(decl) if matches!(decl.name.as_str(), "probe" | "escaping")) {
                continue;
            }
            checker.check_item(item).expect("declarations must independently typecheck");
        }
        assert!(checker.take_deferred_errors().is_empty());
        assert!(checker.diagnostic_sources().is_empty());
        let probe = ast.items.iter().find(|item|
            matches!(&item.kind, ItemKind::Function(decl) if decl.name.name.as_str() == "probe")
        ).unwrap();
        let result = checker.check_item(probe);
        let deferred = checker.take_deferred_errors();
        let diagnostics = checker.diagnostic_sources();
        let passed = result.is_ok() && deferred.is_empty() && diagnostics.is_empty();
        assert_eq!(passed, accepted, "value={value}: {result:?}, {deferred:?}, {diagnostics:?}");
        let escaping = ast.items.iter().find(|item|
            matches!(&item.kind, ItemKind::Function(decl) if decl.name.as_str() == "escaping")
        ).unwrap();
        assert!(matches!(checker.check_item(escaping),
            Err(verum_types::TypeError::DanglingReference { var, .. }) if var.as_str() == "owner"
        ));
    }
}

#[test]
fn a_nominal_future_spelling_and_an_unknown_variable_supply_no_output() {
    use verum_types::ty::TypeVar;
    let checker = TypeChecker::new();
    for ty in [
        Type::Generic { name: "Future".into(), args: List::from_single(Type::Text) },
        Type::Generic { name: "Future".into(), args: List::new() },
        Type::Var(TypeVar::fresh()),
    ] {
        assert!(checker.protocol_checker.read().resolve_future_protocol(&ty).is_none(), "{ty}");
    }
}
