//! Erased tuple newtypes require their declared receiver inside a loop.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    interpreter::Interpreter,
};

#[test]
fn collection_tuple_bindings_resolve_each_nominal_receiver() {
    let source = r#"
        module consumer;
        type Left is (Int);
        type Right is (Int);
        implement Left { fn read(&self) -> Int { self.0 } }
        implement Right { fn read(&self) -> Int { self.0 * 10 } }
        type Rows is { items: List<(Left, Right)> };
        implement Rows {
            fn update(&mut self) {
                for (_, value) in self.items.iter_mut() { *value = Right(5); }
            }
            fn total(&self) -> Int {
                let left = Right(1);
                let mut sum = 0;
                for (left, right) in self.items.iter() {
                    sum = sum + left.read() + right.read();
                }
                sum + left.read()
            }
        }
        fn main() -> Int {
            let mut rows = Rows { items: [(Left(2), Right(4))] };
            rows.update();
            rows.total()
        }
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("consumer"))
        .compile_module(&ast)
        .unwrap();
    let main = module.find_function_by_name("consumer.main").unwrap();
    assert_eq!(
        Interpreter::new(Arc::new(module))
            .execute_function(main)
            .unwrap()
            .try_as_i64(),
        Some(62)
    );
}
