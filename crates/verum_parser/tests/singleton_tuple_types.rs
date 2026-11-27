use verum_ast::{FileId, ItemKind, Module, decl::FunctionParamKind, pretty::format_type};
use verum_common::Text;
use verum_fast_parser::Parser;
use verum_parser::LosslessParser;

fn parameter_type(module: &Module) -> Text {
    let ItemKind::Function(function) = &module.items[0].kind else { panic!("missing function") };
    let FunctionParamKind::Regular { ty, .. } = &function.params[0].kind else { panic!("missing parameter") };
    format_type(ty)
}

#[test]
fn only_commas_owned_by_the_parentheses_construct_tuple_types() {
    for (source_type, expected) in [
        ("(Int)", "Int"),
        ("(Int,)", "(Int,)"),
        ("((Int,))", "(Int,)"),
        ("((Int,),)", "((Int,),)"),
        ("(Int, Bool,)", "(Int, Bool)"),
        ("((Int, Bool))", "(Int, Bool)"),
    ] {
        let source = format!("fn keep(value: {source_type}) {{ }}");
        let compiled = Parser::new(&source).parse_module().unwrap();
        let edited = LosslessParser::new().parse(&source, FileId::new(0));
        assert!(edited.errors.is_empty(), "{source}: {:?}", edited.errors);
        for module in [&compiled, &edited.module] {
            assert_eq!(parameter_type(module), expected, "{source}");
        }
    }
}
