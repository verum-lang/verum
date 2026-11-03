//! Source-associated equalities may leave a projection unresolved. Retrying an
//! identical reduction must not spend the normalizer's entire depth budget.
use verum_ast::{FileId, ItemKind, decl::FunctionParamKind};
use verum_fast_parser::FastParser;
use verum_types::{TypeChecker, ty::Type};

fn register(checker: &mut TypeChecker, source: &str) {
    let ast = FastParser::new()
        .parse_module_str(source, FileId::new(0))
        .unwrap();
    for item in &ast.items {
        match &item.kind {
            ItemKind::Type(decl) => checker.register_type_declaration(decl).unwrap(),
            ItemKind::Impl(decl) => checker.register_impl_block(decl).unwrap(),
            _ => panic!("unexpected fixture declaration"),
        }
    }
}

fn checker(source: &str) -> TypeChecker {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    register(&mut checker, source);
    checker
}

fn annotation(source: &str) -> verum_ast::ty::Type {
    let ast = FastParser::new()
        .parse_module_str(&format!("fn probe(value: {source}) {{}}"), FileId::new(1))
        .unwrap();
    let ItemKind::Function(function) = &ast.items[0].kind else {
        panic!("expected function")
    };
    let FunctionParamKind::Regular { ty, .. } = &function.params[0].kind else {
        panic!("expected parameter")
    };
    ty.clone()
}

#[test]
fn source_projection_preserves_fixed_point() {
    let mut checker = checker(
        "type Source is protocol { type Item; }; type Ring is {}; \
         implement Source for Ring { type Item = Ring.Item; }",
    );
    let projection = annotation("Ring.Item");
    eprintln!("NORMALIZATION_BEGIN");
    let first = checker.ast_to_type(&projection).unwrap();
    let second = checker.ast_to_type(&projection).unwrap();
    eprintln!("NORMALIZATION_END");
    assert_eq!(first, second);
    assert!(matches!(&first, Type::Generic { name, .. } if name.as_str() == "::Item"));
    assert!(!checker.types_equivalent(&first, &Type::Int));
}

#[test]
fn fixed_point_work_is_bounded() {
    // Count the existing resolver trace in an isolated process so parallel tests
    // cannot affect its environment or lazy trace gate. This checks work rather
    // than imposing a machine-dependent timing threshold. Before the progress
    // guard the same two source annotations require 102 resolver calls.
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "source_projection_preserves_fixed_point",
            "--nocapture",
        ])
        .env("VERUM_TRACE_CTOR", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let trace = String::from_utf8(output.stderr).unwrap();
    let measured = trace
        .split_once("NORMALIZATION_BEGIN\n")
        .unwrap()
        .1
        .split_once("NORMALIZATION_END")
        .unwrap()
        .0;
    let lookups = measured
        .lines()
        .filter(|line| line.starts_with("[assoc]"))
        .count();
    assert_eq!(lookups, 2, "each source projection needs one resolver step");
}

#[test]
fn deferred_projection_can_resolve_after_impl_registration() {
    let mut checker = checker("type Source is protocol { type Item; }; type Ring is {};");
    let projection = annotation("Ring.Item");
    let deferred = checker.ast_to_type(&projection).unwrap();
    assert!(matches!(&deferred, Type::Generic { name, .. } if name.as_str() == "::Item"));
    assert!(!checker.types_equivalent(&deferred, &Type::Int));
    register(
        &mut checker,
        "implement Source for Ring { type Item = Int; }",
    );
    assert_eq!(checker.ast_to_type(&projection).unwrap(), Type::Int);
    assert!(checker.types_equivalent(&deferred, &Type::Int));
}

#[test]
fn projection_reduction_retains_generic_arguments() {
    let mut checker = checker(
        "type Source is protocol { type Item; }; type Ring<T> is { value: T }; \
         implement<T> Source for Ring<T> { type Item = T; }",
    );
    for (source, expected) in [("Ring<Int>", Type::Int), ("Ring<Text>", Type::Text)] {
        let receiver = checker.ast_to_type(&annotation(source)).unwrap();
        let projection = Type::Generic {
            name: "::Item".into(),
            args: vec![receiver].into(),
        };
        assert!(checker.types_equivalent(&projection, &expected));
    }
}

#[test]
fn nominal_type_is_not_reinterpreted_as_associated_projection() {
    let mut checker = checker(
        "type Source is protocol { type Item; }; type Ring is {}; \
         type Item<T> is { value: T }; implement Source for Ring { type Item = Int; }",
    );
    let nominal = checker.ast_to_type(&annotation("Item<Ring>")).unwrap();
    assert!(!checker.types_equivalent(&nominal, &Type::Int));
}
