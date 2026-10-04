//! T1530: source-owned nominal paths keep fields and generic method schemes.
use verum_ast::{ItemKind, Module};
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

fn errors(owner: &str, source: &str) -> List<Text> {
    let module = Parser::new(source).parse_module().expect("source syntax");
    let mut checker = TypeChecker::new();
    register(&mut checker, owner, &module);
    check(&mut checker, &module)
}

fn register(checker: &mut TypeChecker, owner: &str, module: &Module) {
    checker.set_current_module_path(owner);
    for item in &module.items {
        if let ItemKind::Type(decl) = &item.kind {
            checker
                .register_type_declaration(decl)
                .expect("type declaration");
        }
    }
    for item in &module.items {
        if let ItemKind::Impl(decl) = &item.kind {
            checker.register_impl_block(decl).expect("impl declaration");
        }
    }
    for item in &module.items {
        if let ItemKind::Function(decl) = &item.kind {
            checker
                .register_function_signature(decl)
                .expect("function signature");
        }
    }
}

fn check(checker: &mut TypeChecker, module: &Module) -> List<Text> {
    let mut errors: List<_> = module
        .items
        .iter()
        .filter_map(|item| {
            checker
                .check_item(item)
                .err()
                .map(|error| Text::from(format!("{error:?}")))
        })
        .collect();
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors
}

#[test]
fn owned_generic_method_result_needs_no_annotation() {
    let source = r#"
        type Answer is { value: Int };
        implement Answer { fn from_value(value: Int) -> Answer { Answer { value } } }
        type Factory<Item> is { value: Item };
        implement<Item> Factory<Item> { fn build<Output>(self) -> Output { Output.from_value(7) } }
        fn probe() -> Int {
            let factory: Factory<Int> = Factory { value: 7 };
            let result = factory.build<Answer>();
            result.value
        }
    "#;
    for owner in ["cog", "fixture", "cog.nested"] {
        let errors = errors(owner, source);
        assert!(errors.is_empty(), "owner={owner}: {errors:?}");
    }
}

#[test]
fn owned_generic_record_field_keeps_instantiated_type() {
    let source =
        "type State<T> is { pending: T }; fn read(value: State<Int>) -> Int { value.pending }";
    for owner in ["cog", "server", "cog.nested"] {
        let errors = errors(owner, source);
        assert!(errors.is_empty(), "owner={owner}: {errors:?}");
    }
}

#[test]
fn owned_record_fields_work_through_references_and_inherent_self() {
    let source = r#"
        type State is { pending: Bool, calls: Int };
        implement State {
            fn advance(&mut self) -> Int {
                self.pending = false;
                self.calls += 1;
                self.calls
            }
        }
        fn read(value: &State) -> Bool { value.pending }
    "#;
    for owner in ["cog", "server", "cog.nested"] {
        let errors = errors(owner, source);
        assert!(errors.is_empty(), "owner={owner}: {errors:?}");
    }
}

#[test]
fn explicit_type_argument_still_rejects_an_unrelated_value() {
    let source = r#"
        type Factory<Item> is { value: Item };
        implement<Item> Factory<Item> { fn choose<Output>(self, value: Output) -> Output { value } }
        fn probe() -> Bool { let factory: Factory<Int> = Factory { value: 7 }; factory.choose<Int>(true) }
    "#;
    for owner in ["cog", "fixture", "cog.nested"] {
        let errors = errors(owner, source);
        assert!(
            errors.iter().any(|e| e.contains("Mismatch")),
            "owner={owner}: {errors:?}"
        );
    }
}

fn sibling_errors(reverse: bool, source: &str) -> List<Text> {
    let alpha = Parser::new(
        r#"
        public type State<T> is { value: T, alpha_only: Int };
        implement<T> State<T> {
            fn choose<Output>(self, value: Output) -> Output { value }
            fn own_value(&self) -> T { self.value }
        }
    "#,
    )
    .parse_module()
    .expect("alpha source");
    let beta = Parser::new(
        r#"
        public type State<T> is { beta_only: Int, value: Bool };
        implement<T> State<T> {
            fn choose(self, value: Bool) -> Bool { value }
            fn own_value(&self) -> Bool { self.value }
            fn only_beta(&self) -> Bool { self.value }
        }
    "#,
    )
    .parse_module()
    .expect("beta source");
    let mut checker = TypeChecker::new();
    if reverse {
        register(&mut checker, "beta", &beta);
        register(&mut checker, "alpha", &alpha);
    } else {
        register(&mut checker, "alpha", &alpha);
        register(&mut checker, "beta", &beta);
    }
    let consumer = Parser::new(source).parse_module().expect("consumer source");
    register(&mut checker, "consumer", &consumer);
    check(&mut checker, &consumer)
}

#[test]
fn sibling_qualified_fields_and_methods_keep_declared_owner_in_both_orders() {
    for reverse in [false, true] {
        let errors = sibling_errors(
            reverse,
            r#"
            fn first(s: &alpha.State<Int>) -> Int { s.value }
            fn second(s: &beta.State<Int>) -> Bool { s.value }
            fn choose_first(s: alpha.State<Int>) -> Int { s.choose<Int>(7) }
            fn choose_second(s: beta.State<Int>) -> Bool { s.choose(true) }
            fn method_first(s: &alpha.State<Int>) -> Int { s.own_value() }
            fn method_second(s: &beta.State<Int>) -> Bool { s.own_value() }
        "#,
        );
        assert!(errors.is_empty(), "reverse={reverse}: {errors:?}");
    }
}

#[test]
fn sibling_method_cannot_replace_explicit_parameter_check() {
    for reverse in [false, true] {
        let errors = sibling_errors(
            reverse,
            "fn probe(s: alpha.State<Int>) -> Bool { s.choose<Int>(true) }",
        );
        assert!(
            errors.iter().any(|e| e.contains("Mismatch")),
            "reverse={reverse}: {errors:?}"
        );
    }
}

#[test]
fn sibling_field_cannot_supply_a_missing_declared_field() {
    for reverse in [false, true] {
        for source in [
            "fn probe(s: alpha.State<Int>) { let missing = s.beta_only; }",
            "fn probe(s: beta.State<Int>) { let missing = s.alpha_only; }",
        ] {
            let errors = sibling_errors(reverse, source);
            assert!(
                errors.iter().any(|e| e.contains("UnknownField")),
                "reverse={reverse}: {errors:?}"
            );
        }
    }
}

#[test]
fn qualified_impl_and_alias_keep_the_foreign_declaration_owner() {
    for reverse in [false, true] {
        let errors = sibling_errors(
            reverse,
            r#"
            type Local<T> is alpha.State<T>;
            implement<T> alpha.State<T> { fn added<Output>(self, value: Output) -> Output { value } }
            fn probe(s: Local<Int>) -> Int { s.added<Int>(7) }
            fn field(s: &Local<Int>) -> Int { s.value }
        "#,
        );
        assert!(errors.is_empty(), "reverse={reverse}: {errors:?}");
    }
}

#[test]
fn qualified_owner_and_method_generic_shadow_keep_distinct_arguments() {
    let source = r#"
        type Factory<Output> is { value: Output };
        implement<Output> Factory<Output> {
            fn choose<Output>(self, value: Output) -> Output { value }
            fn original(self) -> Output { self.value }
        }
        fn probe(factory: Factory<Int>) -> Bool { factory.choose<Bool>(true) }
        fn original(factory: Factory<Int>) -> Int { factory.original() }
    "#;
    for owner in ["cog", "fixture", "cog.nested"] {
        let errors = errors(owner, source);
        assert!(errors.is_empty(), "owner={owner}: {errors:?}");
    }
}

#[test]
fn qualified_local_static_method_and_protocol_self_keep_fields() {
    let source = r#"
        type State is { pending: Bool, calls: Int };
        type Read is protocol { fn read(&self) -> Int; };
        implement State { fn new(calls: Int) -> State { State { pending: false, calls } } }
        implement Read for State { fn read(&self) -> Int { self.calls } }
        fn probe() -> Int { let state = State.new(7); state.read() }
    "#;
    for owner in ["cog", "fixture", "cog.nested"] {
        let errors = errors(owner, source);
        assert!(errors.is_empty(), "owner={owner}: {errors:?}");
    }
}

#[test]
fn qualified_inherent_by_value_receiver_still_consumes_affine_binding() {
    for owner in ["cog", "alpha", "cog.nested"] {
        let consumed = errors(
            owner,
            r#"
            type affine Token is { value: Int };
            implement Token { fn consume(self) {} fn observe(&self) -> Int { self.value } }
            fn probe(token: Token) { token.consume(); token.consume(); }
        "#,
        );
        assert!(
            consumed.iter().any(|e| e.contains("MovedValueUsed")),
            "owner={owner}: {consumed:?}"
        );
        let borrowed = errors(
            owner,
            r#"
            type affine Token is { value: Int };
            implement Token { fn consume(self) {} fn observe(&self) -> Int { self.value } }
            fn probe(token: Token) { token.observe(); token.observe(); token.consume(); }
        "#,
        );
        assert!(borrowed.is_empty(), "owner={owner}: {borrowed:?}");
    }
}

#[test]
fn qualified_protocol_target_and_static_method_keep_resolved_owner() {
    for reverse in [false, true] {
        let errors = sibling_errors(
            reverse,
            r#"
            type Read is protocol { fn read(&self) -> Int; fn construct() -> Self; };
            type Local<T> is alpha.State<T>;
            implement Read for Local<Int> {
                fn read(&self) -> Int { self.value }
                fn construct() -> Self { alpha.State { value: 7, alpha_only: 0 } }
            }
            fn read(s: alpha.State<Int>) -> Int { s.read() }
            fn make() -> alpha.State<Int> { Local.construct() }
        "#,
        );
        assert!(errors.is_empty(), "reverse={reverse}: {errors:?}");
    }
}

#[test]
fn sibling_method_cannot_supply_a_missing_declared_method() {
    for reverse in [false, true] {
        let errors = sibling_errors(
            reverse,
            "fn probe(s: alpha.State<Int>) -> Bool { s.only_beta() }",
        );
        assert!(
            errors.iter().any(|e| e.contains("MethodNotFound")),
            "reverse={reverse}: {errors:?}"
        );
    }
}
