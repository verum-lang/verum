//! Selected call value witnesses preserve the semantic result across bindings.
use verum_common::{Shared, Text};
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, CodegenResult, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    module::VbcModule,
    serialize::serialize_module,
};

const CELL: &str = r#"
type Cell<N: meta Int> is { value: () };
implement<N: meta Int> Cell<N> {
    fn create() -> Self { Self { value: () } }
    fn count(&self) -> Int { N }
}
"#;

fn compile(body: &str) -> CodegenResult<VbcModule> {
    let source: Text = format!("{CELL}\n{body}").into();
    let ast = Parser::new(&source).parse_module().expect("source grammar");
    VbcCodegen::with_config(CodegenConfig::new("constructor_witness").with_validation())
        .compile_module(&ast)
}

fn run(body: &str, expected: i64) {
    let module = compile(body).expect("source lowering");
    let bytes = serialize_module(&module).expect("serialize actual module");
    let restored = deserialize_module(&bytes).expect("restore actual module");
    for module in [module, restored] {
        let entry = module
            .find_function_by_name("constructor_witness.main")
            .or_else(|| module.find_function_by_name("main"))
            .expect("main declaration");
        let value = Interpreter::new(Shared::new(module).into_arc())
            .execute_function(entry)
            .expect("actual VM execution");
        assert_eq!(value.try_as_i64(), Some(expected));
    }
}

#[test]
fn selected_constructor_result_preserves_distinct_value_indices() {
    run(
        "fn main() -> Int { let a = Cell<7>.create(); let b = Cell<19>.create(); b.count() - a.count() }",
        12,
    );
}

#[test]
fn temporary_constructor_result_preserves_its_value_index() {
    run("fn main() -> Int { Cell<29>.create().count() }", 29);
}

#[test]
fn alias_static_receiver_preserves_its_selected_value() {
    run(
        "type Seven is Cell<7>; fn main() -> Int { let cell = Seven.create(); cell.count() }",
        7,
    );
}

#[test]
fn factory_result_need_not_be_the_factory_owner() {
    run(
        r#"
type Factory<N: meta Int> is { value: () };
implement<N: meta Int> Factory<N> {
    fn make() -> Cell<N> { Cell { value: () } }
}
fn main() -> Int { let cell = Factory<23>.make(); cell.count() }
"#,
        23,
    );
}

#[test]
fn caller_value_role_survives_symbolic_constructor_forwarding() {
    run(
        r#"
fn fresh<N: meta Int>(source: Cell<N>) -> Int {
    let cell = Cell<N>.create();
    cell.count()
}
fn main() -> Int { let cell: Cell<17> = Cell { value: () }; fresh(cell) }
"#,
        17,
    );
}

#[test]
fn value_witness_does_not_erase_result_reference() {
    run(
        r#"
fn borrowed<N: meta Int>(cell: &Cell<N>) -> &Cell<N> { cell }
fn main() -> Int {
    let cell: Cell<13> = Cell { value: () };
    let reference = borrowed(&cell);
    reference.count()
}
"#,
        13,
    );
}

#[test]
fn a_type_role_cannot_supply_a_value_witness() {
    for body in [
        "fn main() -> Int { let cell = Cell<Int>.create(); cell.count() }",
        "fn bad<T>(argument: T) -> Int { let cell = Cell<T>.create(); cell.count() } fn main() -> Int { bad(()) }",
    ] {
        let error = compile(body).expect_err("type role cannot become a value");
        assert!(
            format!("{error}").contains("exact declaration-owned call witness"),
            "{error}"
        );
    }
}

#[test]
fn an_absent_value_witness_is_not_invented_from_the_result() {
    let error = compile("fn main() -> Int { let cell = Cell.create(); cell.count() }")
        .expect_err("missing source value cannot become zero or a positional type");
    assert!(
        format!("{error}").contains("exact declaration-owned call witness"),
        "{error}"
    );
}
