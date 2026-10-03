#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

// A minimal protocol environment isolates await lowering from archive loading.
// The runtime objects follow the same ownership shape as core.async.waker.
const ENV: &str = r#"
type Poll<T> is Ready(T) | Pending;
type Waker is { marker: Int };
type Context is { waker: &Waker };
implement Context {
    fn from_waker(waker: &Waker) -> Context { Context { waker: waker } }
}
fn noop_waker() -> Waker { Waker { marker: 0 } }
type Thread is ();
implement Thread { fn yield_now() {} }
// A bare homonym must never be selected as the driver's synchronous yield.
fn yield_now() { panic("wrong yield_now binding"); }
type Future is protocol {
    type Output;
    fn poll(&mut self, cx: &mut Context) -> Poll<Self.Output>;
};
"#;

fn run(body: &str) -> i64 {
    let source = format!("{ENV}\n{body}");
    let ast = Parser::new(&source).parse_module().expect("parse");
    let module = VbcCodegen::with_config(CodegenConfig::new("future_pins"))
        .compile_module(&ast)
        .expect("compile");
    let entry = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n == "probe" || n.ends_with(".probe"))
        })
        .expect("probe")
        .id;
    let value = Interpreter::new(Arc::new(module))
        .execute_function(entry)
        .expect("execute");
    assert!(value.is_int(), "expected Int, got {value:?}");
    value.as_i64()
}

#[test]
fn await_polls_a_future_until_ready_and_preserves_its_mutable_state() {
    assert_eq!(
        run(r#"
type Count is { remaining: Int };
implement Future for Count {
    type Output = Int;
    fn poll(&mut self, _cx: &mut Context) -> Poll<Int> {
        if self.remaining > 0 {
            self.remaining = self.remaining - 1;
            Poll.Pending
        } else { Poll.Ready(7) }
    }
}
async fn probe() -> Int { Count { remaining: 3 }.await }
"#),
        7
    );
}

#[test]
fn await_preserves_result_polarity_and_nested_payloads() {
    assert_eq!(
        run(r#"
type Result<T, E> is Ok(T) | Err(E);
type Answer is { value: Int };
implement Future for Answer {
    type Output = Result<Int, Int>;
    fn poll(&mut self, _cx: &mut Context) -> Poll<Result<Int, Int>> {
        if self.value > 0 { Poll.Ready(Result.Ok(self.value)) }
        else { Poll.Ready(Result.Err(5)) }
    }
}
async fn probe() -> Int {
    let good = match (Answer { value: 7 }.await) { Result.Ok(v) => v, Result.Err(_) => 1000 };
    let bad = match (Answer { value: 0 }.await) { Result.Ok(_) => 1000, Result.Err(v) => v };
    good + bad
}
"#),
        12
    );
}

#[test]
fn await_writes_through_a_reference_captured_by_the_future() {
    assert_eq!(
        run(r#"
type Writer is { target: &mut Int };
implement Future for Writer {
    type Output = Int;
    fn poll(&mut self, _cx: &mut Context) -> Poll<Int> {
        *self.target = 41;
        Poll.Ready(1)
    }
}
async fn probe() -> Int {
    let mut value = 0;
    let count = Writer { target: &mut value }.await;
    value + count
}
"#),
        42
    );
}

#[test]
fn an_inherent_poll_method_does_not_make_a_value_a_future() {
    assert_eq!(
        run(r#"
type Ordinary is { value: Int };
implement Ordinary {
    fn poll(&mut self, _cx: &mut Context) -> Poll<Int> { Poll.Ready(1000) }
}
async fn probe() -> Int { let value = Ordinary { value: 7 }.await; value.value }
"#),
        7
    );
}

#[test]
fn eager_async_results_remain_values() {
    assert_eq!(
        run(r#"
async fn answer() -> Int { 7 }
async fn probe() -> Int { answer().await }
"#),
        7
    );
}

#[test]
fn await_substitutes_multiple_parameters_in_a_nested_output() {
    assert_eq!(
        run(r#"
type Result<T, E> is Ok(T) | Err(E);
type OutputRecord<T> is { padding: Int, answer: T };
type Producer<T, E> is { answer: T, error: E };
implement<T, E> Future for Producer<T, E> {
    type Output = Result<OutputRecord<T>, E>;
    fn poll(&mut self, _cx: &mut Context) -> Poll<Result<OutputRecord<T>, E>> {
        Poll.Ready(Result.Ok(OutputRecord { padding: 41, answer: self.answer }))
    }
}
async fn probe() -> Int {
    let producer: Producer<Int, Bool> = Producer { answer: 7, error: false };
    let output = producer.await;
    match output { Result.Ok(record) => record.answer, Result.Err(_) => 1000 }
}
"#),
        7
    );
}

#[test]
fn a_generic_future_preserves_its_output() {
    assert_eq!(
        run(r#"
type Immediate<T> is { value: T };
implement<T> Future for Immediate<T> {
    type Output = T;
    fn poll(&mut self, _cx: &mut Context) -> Poll<T> { Poll.Ready(self.value) }
}

async fn probe() -> Int {
    let future: Immediate<Int> = Immediate { value: 7 };
    future.await
}
"#),
        7
    );
}

#[test]
fn the_awaited_binding_uses_the_output_record_layout() {
    assert_eq!(
        run(r#"
type OutputRecord is { padding: Int, answer: Int };
type Producer is { answer: Int };
implement Future for Producer {
    type Output = OutputRecord;
    fn poll(&mut self, _cx: &mut Context) -> Poll<OutputRecord> {
        Poll.Ready(OutputRecord { padding: 41, answer: self.answer })
    }
}
async fn probe() -> Int {
    let output = Producer { answer: 7 }.await;
    output.answer
}
"#),
        7
    );
}

#[test]
fn pending_does_not_have_an_arbitrary_poll_count_limit() {
    assert_eq!(
        run(r#"
type Count is { remaining: Int };
implement Future for Count {
    type Output = Int;
    fn poll(&mut self, _cx: &mut Context) -> Poll<Int> {
        if self.remaining > 0 {
            self.remaining = self.remaining - 1;
            Poll.Pending
        } else { Poll.Ready(7) }
    }
}
async fn probe() -> Int { Count { remaining: 10001 }.await }
"#),
        7
    );
}

#[test]
fn await_calls_poll_rather_than_another_two_argument_impl_method() {
    assert_eq!(
        run(r#"
type Answer is { value: Int };
implement Future for Answer {
    type Output = Int;
    fn unrelated(&mut self, _cx: &mut Context) -> Poll<Int> { Poll.Ready(1000) }
    fn poll(&mut self, _cx: &mut Context) -> Poll<Int> { Poll.Ready(self.value) }
}
async fn probe() -> Int { Answer { value: 7 }.await }
"#),
        7
    );
}
