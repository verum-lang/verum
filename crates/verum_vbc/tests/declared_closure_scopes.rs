#![cfg(feature = "codegen")]

use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

fn execute_source_and_wire(source: &str, expected: i64) {
    let ast = verum_fast_parser::Parser::new(source).parse_module().unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    codegen.collect_unit_declarations(&[&ast]).unwrap();
    codegen.resolve_pending_imports();
    codegen.compile_unit_items(&[&ast], ItemFailurePolicy::Strict).unwrap();
    let module = codegen.finalize_module_from_state().unwrap();
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    ).unwrap();
    for module in [module, wire] {
        let entry = module.find_function_by_name("consumer.run").unwrap();
        let actual = Interpreter::new(module.into()).execute_function(entry).unwrap();
        assert_eq!(actual.try_as_i64(), Some(expected));
    }
}

#[test]
fn retained_default_closures_cannot_reuse_a_previous_receivers_telescope() {
    execute_source_and_wire(r#"
        module consumer;
        type Seed is protocol { fn seed() -> Self; };
        type Number is { value: Int };
        implement Seed for Number { fn seed() -> Self { Number { value: 37 } } }
        type Reducer is protocol {
            type Item;
            fn fold<A, F: fn(A, Self.Item) -> A>(self, initial: A, step: F) -> A { initial }
            fn initialized<A: Seed>(self) -> A {
                self.fold(A.seed(), |acc, item| acc)
            }
        };
        type Earlier<T> is { marker: Int };
        implement<T> Reducer for Earlier<T> { type Item = T; }
        type Later is { marker: Int };
        implement Reducer for Later { type Item = Int; }
        fn run() -> Int {
            let result = Later { marker: 0 }.initialized<Number>();
            result.value
        }
    "#, 37);
}

#[test]
fn nested_closures_restore_prior_sibling_callable_declarations() {
    execute_source_and_wire(r#"
        module consumer;
        type Number is { value: Int };
        fn apply<A, F: fn(A) -> A, G: fn(A) -> A>(value: A, first: F, second: G) -> A {
            second(first(value))
        }
        fn run() -> Int {
            let result = apply(Number { value: 41 },
                |value| { let inner = |number: Int| number; value },
                |value| value);
            result.value
        }
    "#, 41);
}
