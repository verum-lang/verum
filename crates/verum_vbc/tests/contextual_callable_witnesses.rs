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
