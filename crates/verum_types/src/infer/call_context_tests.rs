use super::TypeChecker;
use verum_ast::Module;
use verum_fast_parser::Parser;

fn prepare(source: &str) -> (TypeChecker, Module) {
    let module = Parser::new(source).parse_module().expect("source grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    let errors = checker.prepare_module_declarations(&module);
    assert!(errors.is_empty(), "valid declarations: {errors:?}");
    (checker, module)
}

#[test]
fn recover_failures_restore_the_enclosing_return_contract_and_depth() {
    for source in [
        "try {missing} recover |error| {1}",
        "try {missing} recover {_ => 1} finally {}",
    ] {
        for checking in [false, true] {
            for depth in [0, 1] {
                let (mut checker, _) = prepare("");
                checker.try_recover_depth = depth;
                checker.current_function_return_type = verum_common::Maybe::Some(crate::Type::Bool);
                let expression = Parser::new(source).parse_expr().expect(source);
                let result = if checking {
                    checker.check_expr(&expression, &crate::Type::Int)
                } else {
                    checker.synth_expr(&expression)
                };
                result.expect_err("missing try body value");
                assert_eq!(
                    checker.try_recover_depth, depth,
                    "{source}, checking={checking}"
                );
                assert_eq!(
                    checker.current_function_return_type,
                    verum_common::Maybe::Some(crate::Type::Bool),
                    "{source}, checking={checking}"
                );
            }
        }
    }
}

#[test]
fn recover_handler_names_do_not_escape_on_success_or_failure() {
    for source in [
        "try {1} recover |hidden| {2}",
        "try {1} recover {hidden => 2}",
        "try {1} recover |hidden| {missing}",
        "try {1} recover {hidden => missing}",
    ] {
        for checking in [false, true] {
            let (mut checker, _) = prepare("");
            let depth = checker.ctx.env.scope_depth();
            let expression = Parser::new(source).parse_expr().expect(source);
            let result = if checking {
                checker.check_expr(&expression, &crate::Type::Int)
            } else {
                checker.synth_expr(&expression)
            };
            assert_eq!(
                result.is_err(),
                source.contains("missing"),
                "{source}: {result:?}"
            );
            assert_eq!(
                checker.ctx.env.scope_depth(),
                depth,
                "{source}, checking={checking}"
            );
            assert!(
                checker.ctx.env.lookup("hidden").is_none(),
                "{source}, checking={checking}"
            );
        }
    }
}

#[test]
fn recover_guards_are_checked_in_both_inference_directions() {
    for (guard, valid) in [("true", true), ("7", false)] {
        let source = format!("try {{1}} recover {{hidden if {guard} => 2, _ => 3}}");
        for checking in [false, true] {
            let (mut checker, _) = prepare("");
            let expression = Parser::new(&source).parse_expr().expect("guard grammar");
            let result = if checking {
                checker.check_expr(&expression, &crate::Type::Int)
            } else {
                checker.synth_expr(&expression)
            };
            assert_eq!(
                result.is_ok(),
                valid,
                "{source}, checking={checking}: {result:?}"
            );
            assert!(checker.ctx.env.lookup("hidden").is_none());
        }
    }
}

#[test]
fn recover_arms_do_not_share_pattern_bindings() {
    let source = "try {1} recover {hidden if true => 2, _ => hidden}";
    for checking in [false, true] {
        let (mut checker, _) = prepare("");
        let expression = Parser::new(source).parse_expr().unwrap();
        let result = if checking {
            checker.check_expr(&expression, &crate::Type::Int)
        } else {
            checker.synth_expr(&expression)
        };
        assert!(
            matches!(result, Err(crate::TypeError::UnboundVariable { ref name, .. })
            if name == "hidden"),
            "{result:?}"
        );
        assert!(checker.ctx.env.lookup("hidden").is_none());
    }
}

fn check_errors(checker: &mut TypeChecker, item: &verum_ast::Item) -> Vec<String> {
    let mut errors: Vec<_> = checker
        .check_item(item)
        .err()
        .into_iter()
        .map(|error| format!("{error:?}"))
        .collect();
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| format!("{error:?}")),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| format!("{error:?}")),
    );
    checker.clear_diagnostics();
    errors
}

#[test]
fn failed_calls_restore_the_enclosing_argument_context() {
    for source in [
        "fn need(x: Int) -> Int {x} fn bad() {need(true);}",
        "fn need<T>(x: T, y: T) {} fn bad() {need<Int>(1, true);}",
        "fn bad() { (|x: Int| x)(true); }",
        "fn bad(f: fn<T>(T, Int) -> T) {f(1, true);}",
        "type Wrapped is (Int); fn bad() {Wrapped(true);}",
        "type Choice is Entry(Int); fn bad() {Entry(true);}",
        "fn need(x: Int) -> Int {x} fn bad() {need(need(true));}",
        "fn need(x: Int {{fn nested() -> Int {1} nested() == it}}) {} fn bad() {need(1);}",
        "type Cell is {value: Int}; implement Cell {fn need(&self, x: Int) -> Int {x}}
         fn bad(cell: Cell) {cell.need(true);}",
        "type Cell is {value: Int}; implement Cell {
            fn next(&self) -> Cell {Cell {value: self.value}}
            fn need(&self, x: Int) -> Int {x}}
         fn bad(cell: Cell) {cell.next().next().need(true);}",
    ] {
        for previous in [false, true] {
            let (mut checker, module) = prepare(source);
            checker.in_call_arg_context = previous;
            let errors = check_errors(&mut checker, module.items.last().unwrap());
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("Mismatch") || error.contains("local declaration")),
                "expected invalid argument: {source}: {errors:?}"
            );
            assert_eq!(
                checker.in_call_arg_context, previous,
                "{source}: {errors:?}"
            );
        }
    }
}

#[test]
fn a_failed_call_does_not_disable_later_affine_move_checks() {
    let source = "type affine Parcel is {value: Int};
        fn need(x: Int) {}
        fn bad() {need(true);}
        fn twice(original: Parcel) { (original, original); }";
    let (mut checker, module) = prepare(source);
    let (mut fresh, _) = prepare(source);
    assert!(!check_errors(&mut checker, &module.items[2]).is_empty());
    let expected = check_errors(&mut fresh, &module.items[3]);
    let actual = check_errors(&mut checker, &module.items[3]);
    assert!(
        format!("{expected:?}").contains("MovedValueUsed"),
        "{expected:?}"
    );
    assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
}

#[test]
fn valid_nested_calls_preserve_the_enclosing_argument_context() {
    for previous in [false, true] {
        let (mut checker, module) =
            prepare("fn need(x: Int) -> Int {x} fn valid() -> Int {need(need(1))}");
        checker.in_call_arg_context = previous;
        let errors = check_errors(&mut checker, module.items.last().unwrap());
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(checker.in_call_arg_context, previous);
    }
}

#[test]
fn borrowing_methods_do_not_consume_affine_receivers() {
    let (mut checker, module) = prepare(
        "type affine Cell is {value: Int};
         implement Cell {fn read(&self) -> Int {self.value}}
         fn valid(cell: Cell) -> Int {cell.read() + cell.read()}",
    );
    for item in &module.items {
        let errors = check_errors(&mut checker, item);
        assert!(errors.is_empty(), "{errors:?}");
    }
    assert!(!checker.in_call_arg_context);
}

#[test]
fn failed_unsafe_block_does_not_authorize_a_later_invalid_cast() {
    let (mut checker, _) = prepare("");
    let later = Parser::new("true as (Int, Int)").parse_expr().unwrap();
    let control = checker
        .synth_expr(&later)
        .expect_err("invalid cast in safe context");
    assert!(
        matches!(control, crate::TypeError::InvalidCast { .. }),
        "{control:?}"
    );
    let invalid = Parser::new("unsafe {missing}").parse_expr().unwrap();
    checker.synth_expr(&invalid).expect_err("undefined tail");
    let error = checker
        .synth_expr(&later)
        .expect_err("unsafe permission ended with its block");
    assert!(
        matches!(error, crate::TypeError::InvalidCast { .. }),
        "{error:?}"
    );
    assert!(!checker.in_unsafe_context);
}

#[test]
fn failed_async_block_does_not_authorize_a_later_await() {
    let (mut checker, _) = prepare("");
    let invalid = Parser::new("async {missing}").parse_expr().unwrap();
    checker.synth_expr(&invalid).expect_err("undefined tail");
    let later = Parser::new("(1).await").parse_expr().unwrap();
    let error = checker
        .synth_expr(&later)
        .expect_err("await outside async context");
    assert!(
        matches!(error, crate::TypeError::AsyncPropertyViolation { .. }),
        "{error:?}"
    );
    assert!(!checker.in_async_context);
}

#[test]
fn failed_blocks_restore_lexical_bindings() {
    for source in [
        "{let hidden = 1; missing}",
        "unsafe {let hidden = 1; missing}",
        "async {let hidden = 1; missing}",
        "{fn nested(x: Missing) {} 1}",
    ] {
        let (mut checker, _) = prepare("");
        let depth = checker.ctx.env.scope_depth();
        let expression = Parser::new(source).parse_expr().expect(source);
        checker.synth_expr(&expression).expect_err(source);
        assert_eq!(checker.ctx.env.scope_depth(), depth, "{source}");
        assert!(checker.ctx.env.lookup("hidden").is_none(), "{source}");
        assert!(checker.ctx.env.lookup("nested").is_none(), "{source}");
    }
}

#[test]
fn failed_await_restores_the_enclosing_await_marker() {
    for nested in [false, true] {
        let (mut checker, _) = prepare("");
        if nested {
            assert!(!checker.borrow_tracker.enter_await());
        }
        let expression = Parser::new("missing.await").parse_expr().unwrap();
        checker
            .synth_expr(&expression)
            .expect_err("undefined operand");
        let previous = checker.borrow_tracker.enter_await();
        assert_eq!(previous, nested);
        checker.borrow_tracker.exit_await(previous);
    }
}

#[test]
fn valid_nested_blocks_preserve_permission_and_lexical_boundaries() {
    for source in [
        "unsafe {unsafe {1}; true as (Int, Int)}",
        "async {async {1}; (1).await}",
        "{let hidden = 1; {let hidden = 2; hidden} + hidden}",
    ] {
        let (mut checker, _) = prepare("");
        let depth = checker.ctx.env.scope_depth();
        let expression = Parser::new(source).parse_expr().expect(source);
        checker.synth_expr(&expression).expect(source);
        assert_eq!(checker.ctx.env.scope_depth(), depth, "{source}");
        assert!(!checker.in_async_context, "{source}");
        assert!(!checker.in_unsafe_context, "{source}");
        assert!(checker.ctx.env.lookup("hidden").is_none(), "{source}");
    }
}

#[test]
fn failed_closures_restore_lexical_async_and_capture_state() {
    for source in ["|hidden: Int| missing", "async |hidden: Int| missing"] {
        for checking in [false, true] {
            let (mut checker, _) = prepare("");
            let depth = checker.ctx.env.scope_depth();
            let expression = Parser::new(source).parse_expr().expect(source);
            let result = if checking {
                checker.check_expr(
                    &expression,
                    &crate::Type::function_with_async(
                        vec![crate::Type::Int].into(),
                        crate::Type::Int,
                        true,
                    ),
                )
            } else {
                checker.synth_expr(&expression)
            };
            result.expect_err(source);
            assert_eq!(
                checker.ctx.env.scope_depth(),
                depth,
                "{source}, checking={checking}"
            );
            assert!(!checker.in_async_context, "{source}");
            assert!(!checker.borrow_tracker.in_closure(), "{source}");
            assert!(!checker.borrow_tracker.is_captured("missing"), "{source}");
            assert!(checker.ctx.env.lookup("hidden").is_none(), "{source}");
        }
    }
}

#[test]
fn closure_arguments_still_enforce_affine_moves_inside_their_bodies() {
    let (mut checker, module) = prepare(
        "type affine Parcel is {value: Int};
         fn accept(f: fn(Parcel) -> (Parcel, Parcel)) {}
         fn bad() {accept(|item| (item, item));}",
    );
    let errors = check_errors(&mut checker, module.items.last().unwrap());
    assert!(
        errors.iter().any(|error| error.contains("MovedValueUsed")),
        "{errors:?}"
    );
}

#[test]
fn failed_function_bodies_restore_the_enclosing_context() {
    let (mut checker, module) = prepare("async fn broken(hidden: Int) -> Int {missing}");
    let depth = checker.ctx.env.scope_depth();
    assert!(!check_errors(&mut checker, module.items.last().unwrap()).is_empty());
    assert_eq!(checker.ctx.env.scope_depth(), depth);
    assert!(!checker.in_async_context);
    assert!(checker.current_function_name.is_none());
    assert!(checker.current_function_return_type.is_none());
    assert!(checker.ctx.env.lookup("hidden").is_none());
}

#[test]
fn a_failed_local_function_does_not_discard_enclosing_affine_bindings() {
    let (mut checker, module) = prepare(
        "type affine Parcel is {value: Int};
         fn bad(original: Parcel) {fn broken() -> Int {missing} (original, original);}",
    );
    let errors = check_errors(&mut checker, module.items.last().unwrap());
    assert!(
        errors.iter().any(|error| error.contains("MovedValueUsed")),
        "{errors:?}"
    );
}

#[test]
fn valid_callable_scopes_preserve_their_own_and_enclosing_contracts() {
    for source in [
        "fn outer(value: Bool) -> Bool {fn nested() -> Int {1} value}",
        "async fn outer() -> Int {fn nested() -> Int {1} (1).await}",
        "fn main() {let f = async |value: Int| value.await;}",
        "type affine Parcel is {value: Int}; fn accept(f: fn(Parcel) -> Parcel) {}
         fn caller() {accept(|item| item);}",
    ] {
        let (mut checker, module) = prepare(source);
        for item in &module.items {
            let errors = check_errors(&mut checker, item);
            assert!(errors.is_empty(), "{source}: {errors:?}");
        }
    }
    for source in [
        "async fn outer() {let f = |value: Int| value.await;}",
        "fn main() {let f = |value: Int| value.await;}",
        "fn outer() {async fn nested() -> Int {1} (1).await;}",
    ] {
        let (mut checker, module) = prepare(source);
        let errors = check_errors(&mut checker, module.items.last().unwrap());
        assert!(
            errors
                .iter()
                .any(|error| error.contains("AsyncPropertyViolation")),
            "{source}: {errors:?}"
        );
    }
}
