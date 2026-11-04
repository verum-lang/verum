use verum_common::Shared;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};
fn run(source: &str, expected: i64) {
    let ast = Parser::new(source)
        .parse_module()
        .expect("ordinary source grammar");
    let module = VbcCodegen::with_config(CodegenConfig::new("e2e_test").with_validation())
        .compile_module(&ast)
        .expect("ordinary source lowering");
    let wire =
        deserialize_module(&serialize_module(&module).expect("serialize")).expect("deserialize");
    for module in [module, wire] {
        let id = module
            .find_function_by_name("main")
            .or_else(|| module.find_function_by_name("e2e_test.main"))
            .expect("real main");
        let value = Interpreter::new(Shared::new(module).into_arc())
            .execute_function(id)
            .expect("VM execution");
        assert_eq!(value.try_as_i64(), Some(expected));
    }
}
#[test]
fn original_bare_impl_colliding_const() {
    run(
        r#"
            type AllocPageHeader is { next_page: Int };
            implement AllocPageHeader {
                public const SIZE: Int = 64;
            }
            type LargeAllocHeader is { requested_size: Int };
            implement LargeAllocHeader {
                public const SIZE: Int = 64;
            }

            type MyAlloc<const SIZE: Int> is { used: Int };
            implement MyAlloc<SIZE> {
                public fn new() -> MyAlloc<SIZE> {
                    MyAlloc { used: 0 }
                }
                public fn capacity(&self) -> Int {
                    SIZE
                }
            }

            fn main() -> Int {
                let a = MyAlloc<17>.new();
                a.capacity()
            }
            "#,
        17,
    );
}
#[test]
fn bare_meta_receiver_retains_value_role() {
    run(
        r#"
type Cell<N: meta Int> is { value: () };
implement Cell<N> { fn capacity(&self) -> Int { N } }
fn main() -> Int { let cell: Cell<17> = Cell { value: () }; cell.capacity() }
"#,
        17,
    );
}
#[test]
fn concrete_impl_does_not_claim_method_type_as_owner_value() {
    run(
        r#"
type Cell<N: meta Int> is { value: () };
implement Cell<7> { fn echo<N>(x: N) -> N { x } }
fn main() -> Int { Cell<7>.echo<Int>(17) }
"#,
        17,
    );
}
#[test]
fn bare_impl_method_local_type_shadows_value_owner() {
    run(
        r#"
type Cell<N: meta Int> is { value: () };
implement Cell<N> { fn echo<N>(x: N) -> N { x } }
fn main() -> Int { Cell<7>.echo<Int>(17) }
"#,
        17,
    );
}
