//! Field values keep their exact nominal declaration through local captures.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module, serialize::serialize_module, interpreter::Interpreter};

fn run(source: &str) -> Vec<Result<i64, String>> {
    let ast = Parser::new(source).parse_module().unwrap();
    [false, true].into_iter().map(|bootstrap| {
        let mut compiler = VbcCodegen::with_config(CodegenConfig::new("field_evidence"));
        let module = if bootstrap {
            compiler.collect_unit_declarations(&[&ast]).unwrap();
            compiler.compile_unit_items(&[&ast], ItemFailurePolicy::Strict).unwrap();
            compiler.finalize_module_from_state().unwrap()
        } else { compiler.compile_module(&ast).unwrap() };
        let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        let entry = module.find_function_by_name("field_evidence.probe").unwrap();
        Interpreter::new(Arc::new(module)).execute_function(entry)
            .map(|value| value.as_i64()).map_err(|error| error.to_string())
    }).collect()
}

#[test]
fn generic_self_fields_and_nested_records_keep_capture_declarations() {
    for source in [
        "type Cell<T> is { value: T }; implement<T> Cell<T> { fn make(&self) -> fn() -> T { let value = self.value; || value } } fn probe() -> Int { let cell: Cell<Int> = Cell { value: 37 }; let f = cell.make(); f() }",
        "type Cell<T> is { value: T }; type Envelope<T> is { inner: Cell<T> }; implement<T> Envelope<T> { fn make(&self) -> fn() -> T { let value = self.inner.value; || value } } fn probe() -> Int { let cell: Envelope<Int> = Envelope { inner: Cell { value: 37 } }; let f = cell.make(); f() }",
        "type Cell<T> is { value: T }; implement<T> Cell<T> { fn make<T>(&self, other: T) -> fn() -> Int { let value = self.value; || { if value { 37 } else { 0 } } } } fn probe() -> Int { let cell: Cell<Bool> = Cell { value: true }; let f = cell.make<Int>(9); f() }",
    ] {
        assert_eq!(run(source), [Ok(37), Ok(37)], "{source}");
    }
}

#[test]
fn a_field_declaration_never_authorizes_copying_an_affine_value() {
    let results = run("type affine Token is { value: Int }; type Cell<T> is { value: T }; implement<T> Cell<T> { fn make(&self) -> fn() -> T { let value = self.value; || value } } fn probe() -> Int { let cell: Cell<Token> = Cell { value: Token { value: 37 } }; let f = cell.make(); let token = f(); token.value }");
    for result in results {
        let error = result.expect_err("affine field must not be duplicated");
        assert!(error.contains("requires unrestricted ownership") && error.contains("Affine"), "{error}");
    }
}

#[test]
fn ordinary_await_keeps_the_same_generic_field_capture_evidence() {
    assert_eq!(run(r#"
        type Poll<T> is Ready(T) | Pending;
        type Waker is { marker: Int };
        type Context is { waker: &Waker };
        implement Context { fn from_waker(waker: &Waker) -> Context { Context { waker: waker } } }
        fn noop_waker() -> Waker { Waker { marker: 0 } }
        type Thread is ();
        implement Thread { fn yield_now() {} }
        type Future is protocol { type Output; fn poll(&mut self, cx: &mut Context) -> Poll<Self.Output>; };
        type Record<T> is { padding: Int, value: T };
        type Immediate<T> is { value: T };
        implement<T> Future for Immediate<T> {
            type Output = Record<T>;
            fn poll(&mut self, cx: &mut Context) -> Poll<Record<T>> {
                let value = self.value;
                let closure = || value;
                Poll.Ready(Record { padding: 99, value: closure() })
            }
        }
        async fn probe() -> Int {
            let future: Immediate<Int> = Immediate { value: 37 };
            let output = future.await;
            output.value
        }
    "#), [Ok(37), Ok(37)]);
}
