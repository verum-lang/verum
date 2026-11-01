//! Source-to-type correspondence for the checked refinement lattice fragment.
use verum_ast::{FileId, ItemKind, Module, decl::FunctionParamKind};
use verum_types::{TypeChecker, subtype::Subtyping, ty::Type};

const SOURCE: &str = r#"
fn strong(x: Int { (self > 10 && self < 20) || self == 100 }) {}
fn weak(x: Int { (it > 0 || it == 100) && (it < 30 || it == 100) }) {}
fn positive(x: Int { self > 0 }) {}
fn impossible(x: Int { false }) {}
"#;

fn validate(module: Module) {
    let mut checker = TypeChecker::new();
    checker.register_builtins();
    let errors = checker.prepare_module_declarations(&module);
    assert!(errors.is_empty(), "{errors:?}");
    assert!(checker.take_deferred_errors().is_empty());
    let mut resolved = |name: &str| {
        let function = module
            .items
            .iter()
            .find_map(|item| match &item.kind {
                ItemKind::Function(function) if function.name.name.as_str() == name => {
                    Some(function)
                }
                _ => None,
            })
            .unwrap();
        let FunctionParamKind::Regular { ty, .. } = &function.params[0].kind else {
            panic!("expected an annotated parameter");
        };
        checker.ast_to_type(ty).unwrap()
    };
    let strong = resolved("strong");
    let weak = resolved("weak");
    let positive = resolved("positive");
    let impossible = resolved("impossible");
    assert!(matches!(strong, Type::Refined { .. }), "{strong:?}");
    let checker = Subtyping::new();
    assert!(checker.is_subtype(&strong, &weak), "{strong:#?}\n{weak:#?}");
    assert!(!checker.is_subtype(&weak, &strong));
    assert!(!checker.is_subtype(&Type::Int, &positive));
    assert!(!checker.is_subtype(&Type::Int, &impossible));
}

#[test]
fn fast_parser_and_type_lowering_preserve_predicate_subjects() {
    validate(
        verum_fast_parser::VerumParser::new()
            .parse_module_str(SOURCE, FileId::new(1))
            .unwrap(),
    );
}

#[test]
fn recursive_parser_and_type_lowering_preserve_predicate_subjects() {
    validate(verum_parser::Parser::new(SOURCE).parse_module().unwrap());
}
