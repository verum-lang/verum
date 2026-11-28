use super::{Type, TypeChecker};
use verum_fast_parser::Parser;

#[test]
fn structural_copy_checks_every_logical_payload_of_the_exact_collection() {
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    for (owner, source) in [
        ("core.collections.list", "public type List<T> is { buffer: &unsafe T };"),
        ("core.collections.map", "public type Map<K,V> is { keys: &unsafe K, values: &unsafe V };"),
        ("core.collections.deque", "public type Deque<T> is { buffer: &unsafe T };"),
        ("foreign", "public type List<T> is { buffer: &unsafe T };"),
        ("tree", r#"
            public type Node is Leaf(Int) | Children(core.collections.list.List<Node>);
            public type Invalid is Children(core.collections.list.List<Invalid>) | Bad(&mut Int);
            public type Naked is { next: Naked };
            public type Owned is { index: core.collections.map.Map<Int, core.collections.deque.Deque<Node>> };
            public type BadKey is { index: core.collections.map.Map<&mut Int, Int> };
            public type BadValue is { index: core.collections.map.Map<Int, &unsafe Int> };
            public type WrongOwner is { list: foreign.List<Int> };
        "#),
    ] {
        checker.set_current_module_path(owner);
        let ast = Parser::new(source).parse_module().unwrap();
        let errors = checker.prepare_module_declarations(&ast);
        assert!(errors.is_empty(), "{owner}: {errors:?}");
    }
    for (name, allowed) in [
        ("Node", true), ("Owned", true), ("Invalid", false), ("Naked", false),
        ("BadKey", false), ("BadValue", false), ("WrongOwner", false),
    ] {
        let ty = Type::Named {
            path: TypeChecker::text_to_path(&format!("tree.{name}").into()),
            args: Default::default(),
        };
        let result = checker.check_structural_clone(&ty, verum_ast::Span::default());
        assert_eq!(result.is_ok(), allowed, "{name}: {result:?}");
    }
}
