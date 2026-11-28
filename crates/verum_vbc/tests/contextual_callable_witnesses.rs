#![cfg(feature = "codegen")]

use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module_validated,
    interpreter::Interpreter,
    serialize::serialize_module,
};

#[test]
fn a_shadowed_callable_parameter_keeps_the_receivers_residual_result_witness() {
    let source = r#"
        module callback_scope;
        type Adapter<F> is { produce: F };
        implement<F: fn(Int) -> B, B> Adapter<F> {
            fn fold<A, G: fn(A, B) -> A>(self, initial: A, combine: G) -> A {
                combine(initial, (self.produce)(41))
            }
            fn pick<F: fn(&B, &B) -> Bool>(self, first: B, compare: F) -> B {
                self.fold(first, |left, right| {
                    if compare(&left, &right) { left } else { right }
                })
            }
        }
        fn produce(value: Int) -> Int { value + 2 }
        fn greater(left: &Int, right: &Int) -> Bool { *left > *right }
        fn probe() -> Int { Adapter { produce: produce }.pick(17, greater) }
    "#;
    run(source, 43);
}

#[test]
fn the_result_of_a_method_does_not_constrain_its_receiver_factory() {
    run(r#"
        module callback_scope;
        type Cell is { value: Int };
        implement Cell { fn positive(&self) -> Bool { self.value > 0 } }
        fn identity<T>(value: T) -> T { value }
        fn probe() -> Int {
            let cell: Cell = Cell { value: 73 };
            let positive: Bool = identity(cell).positive();
            if positive { cell.value } else { 0 }
        }
    "#, 73);
}

#[test]
fn a_result_only_witness_reaches_both_value_producing_branches() {
    run(r#"
        module callback_scope;
        type Stamp<T> is { number: Int };
        fn stamp<T>(number: Int) -> Stamp<T> {
            Stamp { number: if T.id == Bool.id { number } else { -1 } }
        }
        fn identity<T>(value: T) -> T { value }
        fn choose(first: Bool) -> Int {
            let selected: Stamp<Bool> = if identity(first) {
                let other: Stamp<Int> = stamp(4);
                if other.number == -1 { stamp(7) } else { stamp(0) }
            } else {
                match identity(first) { true => stamp(0), false => (stamp(3)) }
            };
            selected.number
        }
        fn probe() -> Int { choose(true) * 10 + choose(false) }
    "#, 73);
}

fn run(source: &str, expected: i64) {
    let ast = verum_fast_parser::Parser::new(source)
        .parse_module()
        .unwrap();
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new("callback_scope"));
    compiler.collect_unit_declarations(&[&ast]).unwrap();
    compiler
        .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
        .unwrap();
    let module = compiler.finalize_module_from_state().unwrap();
    let wire = deserialize_module_validated(&serialize_module(&module).unwrap()).unwrap();
    for (route, module) in [("source", module), ("wire", wire)] {
        let entry = module
            .find_function_by_name("callback_scope.probe")
            .unwrap();
        let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
        assert_eq!(
            vm.execute_function(entry).expect(route).try_as_i64(),
            Some(expected),
            "{route}"
        );
    }
}

#[test]
fn pattern_payloads_preserve_hidden_implementation_parameters() {
    pattern_payloads(
        "fn(&Self.Item) -> B",
        "transform(&item)",
        "|value: &Int| *value + 4",
    );
}

#[test]
fn consumed_pattern_payloads_can_return_the_selected_callable_result() {
    pattern_payloads(
        "fn(Self.Item) -> B",
        "transform(item)",
        "|value: Int| value + 4",
    );
}

fn pattern_payloads(callback: &str, call: &str, argument: &str) {
    for (shape, make, pattern) in [
        ("(T)", "((self.produce)(41))", "(item)"),
        (
            "{ value: T }",
            "{ value: (self.produce)(41) }",
            "{ value: item }",
        ),
    ] {
        let source = r#"
            module callback_scope;
            type Maybe<T> is None | Some$SHAPE;
            type Producer is protocol {
                type Item;
                fn next(&mut self) -> Maybe<Self.Item>;
                fn transform_first<B, F: $CALLBACK>(&mut self, transform: F) -> B {
                    while let Maybe.Some$PATTERN = self.next() {
                        return $CALL;
                    }
                    panic("producer returned no value")
                }
            };
            type Adapter<F> is { produce: F };
            implement<F: fn(Int) -> B, B> Producer for Adapter<F> {
                type Item = B;
                fn next(&mut self) -> Maybe<B> { Maybe.Some$MAKE }
            }
            fn produce(value: Int) -> Int { value + 2 }
            fn probe() -> Int {
                let mut adapter = Adapter { produce: produce };
                adapter.transform_first($ARGUMENT)
            }
        "#
        .replace("$SHAPE", shape)
        .replace("$MAKE", make)
        .replace("$PATTERN", pattern)
        .replace("$CALLBACK", callback)
        .replace("$CALL", call)
        .replace("$ARGUMENT", argument);
        run(&source, 47);
    }
}

#[test]
fn a_payload_cannot_borrow_a_different_nominal_variants_declaration() {
    let source = r#"
        module callback_scope;
        type First<T> is Some(T);
        type Second<T> is Some(T);
        fn probe() -> Int { match First.Some(43) { Second.Some(value) => value } }
    "#;
    let ast = verum_fast_parser::Parser::new(source)
        .parse_module()
        .unwrap();
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new("callback_scope"));
    compiler.collect_unit_declarations(&[&ast]).unwrap();
    let error = compiler
        .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("no declared tag for variant pattern 'Second::Some'")
            && error.to_string().contains("callback_scope.First<Int>"),
        "{error}"
    );
}

#[test]
fn selected_callback_result_reaches_independent_variant_branches() {
    run(r#"
        module callback_scope;
        type Decision<T,E> is Accept(T) | Reject(E);
        fn validate<T>(flag:Bool, check:fn(Bool)->Decision<T,Int>) -> T {
            match check(flag) { Decision.Accept(value) => value, Decision.Reject(_) => panic("rejected") }
        }
        fn number(flag:Bool) -> Int {
            validate(flag, |ok| if ok { Decision.Accept(73) } else { Decision.Reject(9) })
        }
        fn probe() -> Int { number(true) }
    "#, 73);
}

#[test]
fn conflicting_generic_arguments_identify_the_parameter_and_the_source_operand() {
    let source = "module callback_scope;
        fn pair<T>(first:T, second:T) -> T { first }
        fn probe() -> Int { pair(71, false) }";
    let ast = verum_fast_parser::Parser::new(source).parse_module().unwrap();
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new("callback_scope"));
    compiler.collect_unit_declarations(&[&ast]).unwrap();
    let error = compiler.compile_unit_items(&[&ast], ItemFailurePolicy::Strict).unwrap_err();
    let span = error.span.expect("offending call operand");
    assert_eq!(&source[span.start as usize..span.end as usize], "false");
    let message = error.to_string();
    assert!(message.contains("second") && message.contains("Int") && message.contains("Bool"), "{message}");
}
