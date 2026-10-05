//! Compatibility spellings retain the semantic primitive's checker identity.
use verum_ast::{ItemKind, decl::FunctionParamKind};
use verum_common::{List, Text};
use verum_diagnostics::Severity;
use verum_fast_parser::Parser;
use verum_types::{Type, infer::TypeChecker};

fn signature_type(name: &str) -> Type {
    let source = format!("fn identity(value: {name}) -> {name} {{ value }}");
    let ast = Parser::new(&source)
        .parse_module()
        .expect("signature grammar");
    let ItemKind::Function(function) = &ast.items[0].kind else {
        panic!("function fixture")
    };
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker
        .register_function_signature(function)
        .expect("registered source signature");
    checker
        .check_item(&ast.items[0])
        .expect("source identity body");
    let result = checker
        .ast_to_type(function.return_type.as_ref().unwrap())
        .expect("declared result");
    let FunctionParamKind::Regular { ty, .. } = &function.params[0].kind else {
        panic!("regular parameter")
    };
    assert_eq!(checker.ast_to_type(ty).expect("declared parameter"), result);
    result
}

fn warnings(source: &str) -> List<Text> {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    for item in &ast.items {
        match &item.kind {
            ItemKind::Type(decl) => checker.register_type_declaration(decl).expect("type alias"),
            ItemKind::Function(function) => checker
                .register_function_signature(function)
                .expect("signature"),
            _ => panic!("fixture declaration"),
        }
    }
    for item in &ast.items {
        checker.check_item(item).expect("source body");
    }
    let diagnostics = checker.diagnostics();
    assert!(!diagnostics.iter().any(|d| d.severity() == Severity::Error));
    diagnostics
        .iter()
        .filter(|d| d.severity() == Severity::Warning)
        .map(|d| Text::from(d.message()))
        .collect()
}

#[test]
fn lowercase_float_aliases_preserve_parameter_and_return_width() {
    assert_eq!(signature_type("f32"), signature_type("Float32"));
    assert_eq!(signature_type("f64"), signature_type("Float64"));
    assert_ne!(signature_type("f32"), signature_type("f64"));
}

#[test]
fn narrowing_through_lowercase_and_declared_aliases_warns() {
    for source in [
        "fn narrow(value: f64) -> f32 { value as f32 }",
        "fn narrow(value: Float) -> f32 { value as f32 }",
        "type Small is f32; type Large is f64; fn narrow(value: Large) -> Small { value as Small }",
    ] {
        let diagnostics = warnings(source);
        assert_eq!(diagnostics.len(), 1, "{source}: {diagnostics:?}");
        assert!(
            diagnostics[0].contains("precision") && diagnostics[0].contains("range"),
            "{diagnostics:?}"
        );
        assert!(
            !diagnostics[0].contains("truncates toward zero"),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn lowercase_widening_and_equal_width_casts_remain_quiet() {
    for (from, to) in [
        ("f32", "f64"),
        ("f32", "Float32"),
        ("Float64", "f64"),
        ("f32", "Float"),
    ] {
        assert!(
            warnings(&format!(
                "fn convert(value: {from}) -> {to} {{ value as {to} }}"
            ))
            .is_empty()
        );
    }
}

#[test]
fn integer_aliases_retain_width_and_signedness() {
    for (alias, canonical) in [
        ("i8", "Int8"),
        ("i16", "Int16"),
        ("i32", "Int32"),
        ("i64", "Int64"),
        ("i128", "Int128"),
        ("isize", "ISize"),
        ("u8", "UInt8"),
        ("u16", "UInt16"),
        ("u32", "UInt32"),
        ("u64", "UInt64"),
        ("u128", "UInt128"),
        ("usize", "UIntSize"),
    ] {
        assert_eq!(signature_type(alias), signature_type(canonical), "{alias}");
    }
    assert_ne!(signature_type("i32"), signature_type("i64"));
    assert_ne!(signature_type("i32"), signature_type("u32"));
}
