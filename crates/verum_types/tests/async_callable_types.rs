use verum_ast::FileId;
use verum_fast_parser::FastParser;
use verum_types::computational_properties::{
    ComputationalProperty, PropertyRequirement, PropertySet,
};
use verum_types::{Type, TypeChecker, subtype::Subtyping};
fn resolve(source: &str) -> Type {
    let ast = verum_fast_parser::VerumParser::new()
        .parse_type_str(source, FileId::new(7))
        .unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.ast_to_type(&ast).unwrap()
}
fn check(source: &str) -> Vec<String> {
    let ast = FastParser::new()
        .parse_module_str(source, FileId::new(7))
        .unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    let mut errors = checker.prepare_module_declarations(&ast);
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error);
        }
    }
    errors.extend(checker.take_deferred_errors());
    let mut messages: Vec<String> = errors.iter().map(ToString::to_string).collect();
    messages.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.is_error())
            .map(|diagnostic| diagnostic.message().to_string()),
    );
    messages
}
#[test]
fn callable_mapping_retains_eager_output_and_async_property() {
    let ty = resolve("async fn(Int) -> Bool");
    let Type::Function {
        params,
        return_type,
        properties,
        property_requirement,
        ..
    } = &ty
    else {
        panic!("{ty:?}")
    };
    assert_eq!(params.as_slice(), &[Type::Int]);
    assert_eq!(**return_type, Type::Bool);
    assert!(
        properties.is_none(),
        "a source annotation has no inferred body footprint"
    );
    assert_eq!(
        *property_requirement,
        PropertyRequirement::source_callable(true)
    );
    assert_ne!(ty, resolve("fn(Int) -> Bool"));
    assert_eq!(ty.to_string(), "async fn(Int) -> Bool");
}
#[test]
fn rank2_mapping_keeps_the_local_binder_and_async_contract() {
    let ty = resolve("async fn<T>(T) -> T");
    let Type::Forall { vars, body } = ty else {
        panic!("rank2")
    };
    let Type::Function {
        params,
        return_type,
        properties,
        property_requirement,
        ..
    } = *body
    else {
        panic!("function")
    };
    assert!(
        properties.is_none(),
        "a source annotation has no inferred body footprint"
    );
    assert_eq!(
        property_requirement,
        PropertyRequirement::source_callable(true)
    );
    assert_eq!(vars.len(), 1);
    assert_eq!(params[0], *return_type);
    assert_eq!(params[0], Type::Var(vars[0]));
}
#[test]
fn existing_property_subtyping_allows_sync_as_async_but_not_the_reverse() {
    let sync = resolve("fn(Int) -> Int");
    let asynchronous = resolve("async fn(Int) -> Int");
    let checker = Subtyping::new();
    assert!(checker.is_subtype(&sync, &asynchronous));
    assert!(!checker.is_subtype(&asynchronous, &sync));
}
#[test]
fn source_arguments_preserve_the_same_directional_contract() {
    for source in [
        "fn accept(f: async fn() -> Int) -> Int { f() } fn ready() -> Int { 17 } fn good() -> Int { accept(ready) }",
        "fn accept(f: async fn() -> Int) -> Int { f() } async fn ready() -> Int { 17 } fn good() -> Int { accept(ready) }",
        "fn accept(f: async fn() -> Int) -> Int { f() } fn good() -> Int { accept(async || { 17 }) }",
    ] {
        let errors = check(source);
        assert!(errors.is_empty(), "{source}: {errors:?}");
    }
    for source in [
        "fn accept(f: fn() -> Int) -> Int { f() } async fn ready() -> Int { 17 } fn bad() -> Int { accept(ready) }",
        "fn accept(f: fn() -> Int) -> Int { f() } fn bad() -> Int { accept(async || { 17 }) }",
        "async fn ready() -> Int { 17 } fn bad() -> Int { let f: fn() -> Int = ready; f() }",
    ] {
        let errors = check(source);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("expected 'fn(") && error.contains("found 'async fn(")),
            "{source}: {errors:?}"
        );
    }
}

#[test]
fn mutable_callbacks_do_not_need_an_invented_purity_promise() {
    for prefix in ["", "async "] {
        let source = format!(
            "fn accept(f: async fn(&mut Int) -> Int, n: &mut Int) -> Int {{ f(n) }} {prefix}fn bump(n: &mut Int) -> Int {{ *n = *n + 1; *n }} fn good() -> Int {{ let mut n = 0; accept(bump, &mut n) }}"
        );
        let errors = check(&source);
        assert!(errors.is_empty(), "{errors:?}");
    }
    let errors = check(
        "fn accept(f: async fn(&mut Int) -> Int, n: &mut Int) -> Int { f(n) } fn good() -> Int { let mut n = 0; accept(async |n: &mut Int| -> Int { *n = *n + 1; *n }, &mut n) }",
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn nested_annotations_do_not_erase_the_callable_contract() {
    for source in [
        "async fn ready() -> Int { 17 } fn bad() -> Int { let f: &fn() -> Int = &ready; 0 }",
        "async fn ready() -> Int { 17 } fn bad() -> Int { let f: (fn() -> Int, Int) = (ready, 0); 0 }",
        "type Sync is fn() -> Int; async fn ready() -> Int { 17 } fn bad() -> Int { let f: &Sync = &ready; 0 }",
    ] {
        let errors = check(source);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("Type mismatch") && error.contains("async fn")),
            "{source}: {errors:?}"
        );
    }
    for source in [
        "fn ready() -> Int { 17 } fn good() -> Int { let f: &async fn() -> Int = &ready; 0 }",
        "fn ready() -> Int { 17 } fn good() -> Int { let f: (async fn() -> Int, Int) = (ready, 0); 0 }",
    ] {
        let errors = check(source);
        assert!(errors.is_empty(), "{source}: {errors:?}");
    }
}

#[test]
fn an_unknown_footprint_never_establishes_purity() {
    let pure = PropertyRequirement::AtMost(PropertySet::pure());
    let sync = PropertyRequirement::source_callable(false);
    let asynchronous = PropertyRequirement::source_callable(true);
    assert!(!pure.satisfied_by(&sync, None));
    assert!(!pure.satisfied_by(&asynchronous, None));
    assert!(sync.accepts(&PropertySet::single(ComputationalProperty::Mutates)));
    assert!(!sync.accepts(&PropertySet::single(ComputationalProperty::Async)));
    assert!(asynchronous.satisfied_by(&sync, None));
    assert!(!sync.satisfied_by(&asynchronous, None));
    assert!(pure.accepts(&PropertySet::pure()));
    assert!(!pure.accepts(&PropertySet::single(ComputationalProperty::IO)));
    let excludes_pure = PropertyRequirement::Excludes(PropertySet::pure());
    assert!(excludes_pure.accepts(&PropertySet::pure()));
    assert!(asynchronous.implies(&excludes_pure));
}

#[test]
fn unknown_callback_body_cannot_establish_purity() {
    for body in [
        "callback(counter)",
        "let alias = callback; alias(counter)",
        "let thunk = || -> Int { callback(counter) }; thunk()",
    ] {
        let source = format!(
            "pure fn bridge(callback: fn(&mut Int) -> Int, counter: &mut Int) -> Int {{ {body} }} fn bump(counter: &mut Int) -> Int {{ *counter = *counter + 1; *counter }} fn probe() -> Int {{ let mut count = 0; bridge(bump, &mut count) }}"
        );
        let errors = check(&source);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("pure") || error.contains("computational")),
            "{source}: {errors:?}"
        );
    }
}

#[test]
fn inferred_async_and_unknown_callables_cannot_be_coerced_to_strict_fn_types() {
    for body in ["callback()", "let alias = callback; alias()"] {
        let source = format!(
            "fn bridge(callback: async fn() -> Int) -> Int {{ {body} }} fn require(callback: fn(async fn() -> Int) -> Int) {{}} fn probe() {{ require(bridge); }}"
        );
        let errors = check(&source);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("Type mismatch") && error.contains("async fn")),
            "{source}: {errors:?}"
        );
    }
    let errors = check(
        "fn bridge(callback: async fn() -> Int) -> Int { let thunk: fn() -> Int = || -> Int { callback() }; thunk() }",
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("Type mismatch") && error.contains("async fn")),
        "{errors:?}"
    );
}

#[test]
fn ordinary_function_bodies_keep_the_ratified_eager_async_call_law() {
    for source in [
        "async fn ready() -> Int { 17 } fn main() -> Int { ready() }",
        "fn bridge(callback: async fn() -> Int) -> Int { callback() } async fn ready() -> Int { 17 } fn main() -> Int { bridge(ready) }",
        "fn bridge(callback: async fn() -> Int) -> Int { let thunk = || -> Int { callback() }; thunk() }",
    ] {
        let errors = check(source);
        assert!(errors.is_empty(), "{source}: {errors:?}");
    }
}

#[test]
fn known_pure_calls_and_unexecuted_closures_remain_pure() {
    for source in [
        "pure fn identity(value: Int) -> Int { value } pure fn call() -> Int { identity(17) }",
        "pure fn make() -> Int { let callback = |n: &mut Int| -> Int { *n = *n + 1; *n }; 17 }",
    ] {
        let errors = check(source);
        assert!(errors.is_empty(), "{source}: {errors:?}");
    }
}

#[test]
fn constructing_a_function_shape_does_not_prove_its_body_properties() {
    use verum_common::List;
    use verum_types::{TypeVar, di::requirement::ContextRequirement};
    let unknown = [
        Type::function(List::new(), Type::Int),
        Type::function_with_contexts(List::new(), Type::Int, ContextRequirement::empty()),
        Type::function_with_context_var(List::new(), Type::Int, TypeVar::fresh()),
    ];
    let pure = PropertyRequirement::AtMost(PropertySet::pure());
    let synchronous = PropertyRequirement::source_callable(false);
    for ty in unknown {
        let Type::Function {
            property_requirement,
            properties,
            ..
        } = &ty
        else {
            panic!("{ty:?}")
        };
        assert_eq!(property_requirement, &PropertyRequirement::Unrestricted);
        assert!(
            properties.is_none(),
            "a function shape supplies no body evidence"
        );
        assert!(!pure.satisfied_by(property_requirement, properties.as_ref()));
        assert!(!synchronous.satisfied_by(property_requirement, properties.as_ref()));
    }
    let Type::Function {
        properties,
        property_requirement,
        ..
    } = Type::function_with_properties(List::new(), Type::Int, PropertySet::pure())
    else {
        panic!("function")
    };
    assert!(pure.satisfied_by(&property_requirement, properties.as_ref()));
}

#[test]
fn aggregate_conversion_preserves_context_requirements() {
    for wrapper in [
        "(Int, {callback})",
        "&{callback}",
        "{ callback: {callback} }",
    ] {
        let contextual = wrapper.replace("{callback}", "(fn() -> Int using [Log])");
        let plain = wrapper.replace("{callback}", "(fn() -> Int)");
        let prelude = "context Log { fn note(message: Text); }";
        let positive =
            format!("{prelude} fn keep(value: {contextual}) -> ({contextual}) {{ value }}");
        let errors = check(&positive);
        assert!(errors.is_empty(), "{positive}: {errors:?}");
        let negative = format!("{prelude} fn erase(value: {contextual}) -> ({plain}) {{ value }}");
        let errors = check(&negative);
        assert!(
            errors
                .iter()
                .any(|error| (error.contains("Type mismatch") || error.contains("context mismatch"))
                    && error.contains("Log")),
            "{negative}: {errors:?}"
        );
    }
}
