//! Checked array-count expressions use the producer's scalar domain.
use verum_ast::ItemKind;
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    for item in &ast.items {
        match &item.kind {
            ItemKind::Function(function) => checker
                .register_function_signature(function)
                .expect("signature"),
            ItemKind::Const(decl) => checker.pre_register_const(decl),
            _ => (),
        }
    }
    let mut errors: List<_> = ast
        .items
        .iter()
        .filter_map(|item| {
            checker
                .check_item(item)
                .err()
                .map(|e| Text::from(format!("{e:?}")))
        })
        .collect();
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors
}

#[test]
fn nearer_module_owns_a_qualified_count_even_without_the_member() {
    for (inner, count, accepted) in [
        ("", "ns.CAP", true),
        ("module ns { public const CAP: Int = 5; }", "ns.CAP", true),
        ("module ns {}", "ns.CAP", false),
        ("module ns {}", "cog.outer.ns.CAP", true),
        ("module ns {}", "cog.outer.inner.ns.CAP", false),
        ("const ns: Int = 9;", "ns.CAP", true),
    ] {
        let source = format!(
            "module outer {{ module ns {{ public const CAP: Int = 3; }} module inner {{ {inner} fn size<T>()->Int {{T.size}} fn probe()->Int {{size<[Byte; {count}]>()}} }} }}"
        );
        let diagnostics = errors(&source);
        assert_eq!(
            diagnostics.is_empty(),
            accepted,
            "{source}: {diagnostics:?}"
        );
    }
}

#[test]
fn owner_head_shadowing_cannot_fall_through_a_missing_middle_segment() {
    let source = "module outer { module ns { module deep { public const CAP: Int = 3; } } module inner { module ns {} fn size<T>()->Int {T.size} fn probe()->Int {size<[Byte; ns.deep.CAP]>()} } }";
    assert!(
        !errors(source).is_empty(),
        "nearer empty ns must own the missing deep member"
    );
}

#[test]
fn a_later_empty_module_still_owns_its_missing_member() {
    let source = "module outer { module ns { public const CAP: Int = 3; } module inner { fn size<T>()->Int {T.size} fn probe()->Int {size<[Byte; ns.CAP]>()} module ns {} } }";
    assert!(!errors(source).is_empty());
}

#[test]
fn selected_owner_has_the_exact_declared_count() {
    let source = "module outer { module ns { public const CAP: Int = 3; } module inner { const ns: Int = 9; module ns { public const CAP: Int = 5; } } }";
    let ast = Parser::new(source).parse_module().unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    for item in &ast.items {
        checker.check_item(item).unwrap();
    }
    checker.set_current_module_path("cog.outer.inner");
    for (count, expected) in [("ns.CAP", 5), ("cog.outer.ns.CAP", 3)] {
        let syntax = Parser::new(&format!("[Byte; {count}]"))
            .parse_type()
            .unwrap();
        let verum_types::ty::Type::Array { size, .. } = checker.ast_to_type(&syntax).unwrap()
        else {
            panic!("array")
        };
        assert_eq!(size, Some(expected));
    }
}

#[test]
fn module_namespace_precedes_type_layout_properties() {
    for member in ["public const size: Int = 3;", ""] {
        let source = format!(
            "type ns is {{ value: Int }}; module ns {{ {member} }} fn measure<T>()->Int {{T.size}} fn probe()->Int {{measure<[Byte; ns.size]>()}}"
        );
        let diagnostics = errors(&source);
        assert_eq!(
            diagnostics.is_empty(),
            !member.is_empty(),
            "{source}: {diagnostics:?}"
        );
    }
}

#[test]
fn forward_module_declaration_owns_its_missing_count() {
    let source = "module outer { module ns { public const CAP: Int=3; } module inner { module ns; fn measure<T>()->Int {T.size} fn probe()->Int {measure<[Byte; ns.CAP]>()} } }";
    assert!(
        !errors(source).is_empty(),
        "forward module must block outer owner"
    );
}
