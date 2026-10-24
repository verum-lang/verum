//! Intrinsic spawned futures must not borrow a library record's nominal name.
use verum_ast::{FileId, ItemKind};
use verum_common::Text;
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
