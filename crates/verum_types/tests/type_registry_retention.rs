use verum_ast::{FileId, Span};
use verum_common::Text;
use verum_types::{Type, TypeRegistry};

fn span(index: u32) -> Span {
    Span::new(index * 2, index * 2 + 1, FileId::new(1))
}

#[test]
fn expression_facts_survive_a_large_compilation_unit() {
    let mut registry = TypeRegistry::new();
    for index in 0..100_002 { registry.register_expr(span(index), Type::Int); }
    registry.register_expr(span(50_000), Type::Bool);
    assert_eq!(registry.stats().0, 100_002);
    for index in [0, 50_000, 99_999, 100_000, 100_001] {
        let expected = if index == 50_000 { &Type::Bool } else { &Type::Int };
        assert_eq!(registry.lookup_expr(span(index)), Some(expected));
    }
    registry.clear();
    assert_eq!(registry.stats(), (0, 0, 0));
}

#[test]
fn variable_facts_survive_a_large_compilation_unit() {
    let mut registry = TypeRegistry::new();
    for index in 0..50_002 { registry.register_var(span(index), "value".into(), Type::Bool); }
    assert_eq!(registry.stats().1, 50_002);
    let name = Text::from("value");
    for index in [0, 25_000, 49_999, 50_000, 50_001] {
        assert_eq!(registry.lookup_var(span(index), &name), Some(&Type::Bool));
    }
}

#[test]
fn return_facts_survive_a_large_compilation_unit() {
    let mut registry = TypeRegistry::new();
    for index in 0..10_002 { registry.register_func_return(span(index), Type::Text); }
    assert_eq!(registry.stats().2, 10_002);
    for index in [0, 5_000, 9_999, 10_000, 10_001] {
        assert_eq!(registry.lookup_func_return(span(index)), Some(&Type::Text));
    }
}
