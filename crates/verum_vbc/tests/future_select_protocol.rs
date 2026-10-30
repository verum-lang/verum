#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};
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

fn run(body: &str, expected: i64) {
    let source = format!("{ENV}\n{body}");
    let ast = Parser::new(&source).parse_module().expect("parse");
    for bootstrap in [false, true] {
        let mut compiler = VbcCodegen::with_config(CodegenConfig::new("select_pins"));
        let module = if bootstrap {
            compiler
                .collect_unit_declarations(&[&ast])
                .expect("declarations");
            compiler
                .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
                .expect("items");
            compiler.finalize_module_from_state().unwrap()
        } else {
            compiler.compile_module(&ast).expect("compile")
        };
        let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        let entry = module.find_function_by_name("select_pins.probe").unwrap();
        let value = Interpreter::new(Arc::new(module))
            .execute_function(entry)
            .expect("execute");
        assert!(value.is_int(), "expected Int, got {value:?}");
        assert_eq!(value.as_i64(), expected, "bootstrap={bootstrap}");
    }
}
const FUTURES: &str = r#"
type Count is { remaining: Int, value: Int };
implement Future for Count {
    type Output = Int;
    fn poll(&mut self, _cx: &mut Context) -> Poll<Int> {
        if self.remaining < 0 { panic("losing future polled after winner"); }
        if self.remaining > 0 { self.remaining = self.remaining - 1; Poll.Pending }
        else { self.remaining = -1; Poll.Ready(self.value) }
    }
}
type Never is { polls: Int };
implement Future for Never {
    type Output = Int;
    fn poll(&mut self, _cx: &mut Context) -> Poll<Int> {
        self.polls = self.polls + 1;
        assert(self.polls < 8);
        Poll.Pending
    }
}
"#;
#[test]
fn pending_loser_does_not_block_ready_winner_in_either_position() {
    for arms in [
        "a = Never { polls: 0 }.await => a, b = Count { remaining: 2, value: 17 }.await => b",
        "b = Count { remaining: 2, value: 17 }.await => b, a = Never { polls: 0 }.await => a",
    ] {
        run(
            &format!("{FUTURES} async fn probe() -> Int {{ select biased {{ {arms} }} }}"),
            17,
        );
    }
}
#[test]
fn ready_winner_does_not_poll_later_arms() {
    run(
        &format!(
            "{FUTURES} async fn probe() -> Int {{ select biased {{ a = Count {{ remaining: 0, value: 11 }}.await => a, b = Count {{ remaining: -1, value: 0 }}.await => b }} }}"
        ),
        11,
    );
}
#[test]
fn disabled_arm_is_not_constructed_and_else_observes_one_pending_sweep() {
    run(
        &format!(
            r#"{FUTURES}
fn disabled() -> Never {{ panic("disabled operand was constructed"); }}
async fn probe() -> Int {{ select {{
    a = disabled().await if false => a,
    b = Never {{ polls: 0 }}.await => b,
    else => 23,
}} }}"#
        ),
        23,
    );
}
#[test]
fn pattern_mismatch_disables_completed_operand_without_repolling() {
    run(
        &format!(
            r#"{FUTURES} async fn probe() -> Int {{ select biased {{
        9 = Count {{ remaining: 0, value: 1 }}.await => 1000,
        b = Count {{ remaining: 2, value: 17 }}.await => b,
    }} }}"#
        ),
        17,
    );
}
#[test]
fn generic_poll_and_nested_output_keep_declaration_witnesses() {
    run(
        r#"
type Record<T> is { padding: Int, value: T };
type Immediate<T> is { value: T };
implement<T> Future for Immediate<T> {
    type Output = Record<T>;
    fn poll(&mut self, _cx: &mut Context) -> Poll<Record<T>> {
        let value = self.value;
        let closure = || value;
        Poll.Ready(Record { padding: 99, value: closure() })
    }
}
async fn probe() -> Int {
    let future: Immediate<Int> = Immediate { value: 37 };
    select { output = future.await => output.value }
}
"#,
        37,
    );
}
#[test]
fn keyword_tasks_remain_selectable() {
    run(
        "async fn probe() -> Int { let handle = spawn { 41 }; select { result = handle.await => result } }",
        41,
    );
}
#[test]
fn unsupported_eager_async_call_is_refused_before_execution() {
    let source = format!(
        "{ENV} async fn eager() -> Int {{ loop {{}} }} async fn probe() -> Int {{ select {{ x = eager().await => x }} }}"
    );
    let ast = Parser::new(&source).parse_module().unwrap();
    let error = VbcCodegen::with_config(CodegenConfig::new("invalid_select"))
        .compile_module(&ast)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("eager async calls are not selectable"),
        "{error}"
    );
}

#[test]
fn temporary_losers_are_dropped_before_returning_arm_but_named_futures_stay_live() {
    run(
        r#"
type Tracked is { drops: &mut Int, pending: Bool };
implement Drop for Tracked { fn drop(&mut self) { *self.drops = *self.drops + 1; } }
implement Future for Tracked {
    type Output = Int;
    fn poll(&mut self, _cx: &mut Context) -> Poll<Int> {
        if self.pending { Poll.Pending } else { Poll.Ready(5) }
    }
}
async fn select_once(drops: &mut Int) -> Int {
    select biased {
        a = Tracked { drops: drops, pending: true }.await => a,
        b = Tracked { drops: drops, pending: false }.await => return *drops,
    }
}
async fn probe() -> Int {
    let mut drops = 0;
    let value = select_once(&mut drops).await;
    let held = Tracked { drops: &mut drops, pending: false };
    let ready = select { answer = held.await => answer };
    value * 100 + drops * 10 + ready
}
"#,
        225,
    );
}

#[test]
fn selected_borrow_preserves_whole_self_replacement() {
    run(
        r#"
type Replace is { value: Int };
implement Future for Replace {
    type Output = Int;
    fn poll(&mut self, _cx: &mut Context) -> Poll<Int> { *self = Replace { value: 17 }; Poll.Ready(41) }
}
async fn probe() -> Int {
    let mut future = Replace { value: 9 };
    let result = select biased { value = future.await => value };
    result + future.value
}
"#,
        58,
    );
}
#[test]
fn pattern_mismatch_drops_completed_output() {
    run(
        r#"
type Maybe<T> is None | Some(T);
type Guard is { drops: &mut Int };
implement Drop for Guard { fn drop(&mut self) { *self.drops = *self.drops + 1; } }
type Maker is { drops: &mut Int };
implement Future for Maker {
    type Output = Maybe<Guard>;
    fn poll(&mut self, _cx: &mut Context) -> Poll<Maybe<Guard>> {
        Poll.Ready(Maybe.Some(Guard { drops: self.drops }))
    }
}
async fn probe() -> Int {
    let mut drops = 0;
    select biased { Maybe.None = Maker { drops: &mut drops }.await => 1000, else => drops }
}
"#,
        1,
    );
}
