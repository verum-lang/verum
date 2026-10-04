//! T1539: a block hands its tail value to the caller before local cleanup.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    interpreter::Interpreter,
};

const DECLARATIONS: &str = r#"
type Count is { value: Int };
type Watch is { counter: &mut Count, digit: Int };
implement Drop for Watch {
    fn drop(&mut self) { self.counter.value = self.counter.value * 10 + self.digit; }
}
"#;

fn run(source: &str) -> i64 {
    let ast = Parser::new(&format!("{DECLARATIONS}\n{source}"))
        .parse_module()
        .expect("parse");
    let module = VbcCodegen::with_config(CodegenConfig::new("block_lifetime"))
        .compile_module(&ast)
        .expect("compile");
    let probe = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|name| name.ends_with(".probe"))
        })
        .expect("probe")
        .id;
    Interpreter::new(Arc::new(module))
        .execute_function(probe)
        .expect("execute")
        .as_i64()
}

#[test]
fn returning_a_local_transfers_its_drop_obligation_to_the_caller() {
    assert_eq!(
        run(r#"
fn make(counter: &mut Count) -> Watch {
    let value = Watch { counter, digit: 1 };
    value
}
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    { let value = make(&mut counter); if counter.value != 0 { return 99; } }
    counter.value
}
"#),
        1
    );
}

#[test]
fn nested_block_tails_remain_live_until_the_receiving_scope_ends() {
    assert_eq!(
        run(r#"
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    {
        let value = { let inner = Watch { counter: &mut counter, digit: 2 }; inner };
        if counter.value != 0 { return 99; }
    }
    counter.value
}
"#),
        2
    );
}

#[test]
fn returning_one_local_preserves_reverse_drop_order_for_other_locals() {
    assert_eq!(
        run(r#"
fn make(counter: &mut Count) -> Watch {
    let first = Watch { counter, digit: 1 };
    let returned = Watch { counter, digit: 3 };
    let last = Watch { counter, digit: 2 };
    returned
}
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    { let value = make(&mut counter); if counter.value != 21 { return 99; } }
    counter.value
}
"#),
        213
    );
}

#[test]
fn explicit_drop_of_a_returned_local_runs_once() {
    assert_eq!(
        run(r#"
fn make(counter: &mut Count) -> Watch {
    let value = Watch { counter, digit: 4 };
    value
}
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    { let value = make(&mut counter); drop(value); }
    counter.value
}
"#),
        4
    );
}

#[test]
fn discarded_forwarding_expressions_keep_scope_cleanup() {
    for expression in [
        "{ let value = Watch { counter: &mut counter, digit: 1 }; value }",
        "({ let value = Watch { counter: &mut counter, digit: 1 }; value })",
        "unsafe { let value = Watch { counter: &mut counter, digit: 1 }; value }",
        "{ { let value = Watch { counter: &mut counter, digit: 1 }; value } }",
        "if true { let value = Watch { counter: &mut counter, digit: 1 }; value } else { let value = Watch { counter: &mut counter, digit: 2 }; value }",
        "if false { let value = Watch { counter: &mut counter, digit: 2 }; value } else { let value = Watch { counter: &mut counter, digit: 1 }; value }",
        "match true { true => { let value = Watch { counter: &mut counter, digit: 1 }; value }, false => { let value = Watch { counter: &mut counter, digit: 2 }; value } }",
    ] {
        let source = format!(
            "fn probe() -> Int {{ let mut counter = Count {{ value: 0 }}; {expression}; counter.value }}"
        );
        assert_eq!(run(&source), 1, "discarded {expression}");
    }
}

#[test]
fn loop_body_tails_are_discarded_on_each_iteration() {
    for loop_expr in [
        "while i < 2 { i += 1; let value = Watch { counter: &mut counter, digit: 1 }; value }",
        "for n in 0..2 { let value = Watch { counter: &mut counter, digit: 1 }; value }",
        "loop { if i == 2 { break; }; i += 1; let value = Watch { counter: &mut counter, digit: 1 }; value }",
    ] {
        let source = format!(
            "fn probe() -> Int {{ let mut counter = Count {{ value: 0 }}; let mut i = 0; {loop_expr}; counter.value }}"
        );
        assert_eq!(run(&source), 11, "discarded {loop_expr}");
    }
}

#[test]
fn discarded_borrowed_tail_does_not_drop_its_referent() {
    assert_eq!(
        run(r#"
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    {
        let value = Watch { counter: &mut counter, digit: 1 };
        { let borrowed = &value; borrowed };
        if counter.value != 0 { return 99; }
    }
    counter.value
}
"#),
        1
    );
}

#[test]
fn discarded_raw_tail_does_not_drop_its_referent() {
    assert_eq!(
        run(r#"
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    {
        let value = Watch { counter: &mut counter, digit: 1 };
        unsafe { let raw = &value as *const Watch; raw };
        if counter.value != 0 { return 99; }
    }
    counter.value
}
"#),
        1
    );
}

#[test]
fn used_forwarding_results_keep_the_selected_local_alive() {
    for expression in [
        "({ let value = Watch { counter, digit: 1 }; value })",
        "unsafe { let value = Watch { counter, digit: 1 }; value }",
        "if true { let value = Watch { counter, digit: 1 }; value } else { let value = Watch { counter, digit: 2 }; value }",
        "if false { let value = Watch { counter, digit: 2 }; value } else { let value = Watch { counter, digit: 1 }; value }",
        "match true { true => { let value = Watch { counter, digit: 1 }; value }, false => { let value = Watch { counter, digit: 2 }; value } }",
    ] {
        let source = format!(
            r#"
fn make(counter: &mut Count) -> Watch {{ {expression} }}
fn probe() -> Int {{
    let mut counter = Count {{ value: 0 }};
    {{ let value = make(&mut counter); if counter.value != 0 {{ return 99; }} }}
    counter.value
}}
"#
        );
        assert_eq!(run(&source), 1, "used {expression}");
    }
}

#[test]
fn discarded_loop_break_value_keeps_its_local_cleanup() {
    assert_eq!(
        run(r#"
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    loop { break { let value = Watch { counter: &mut counter, digit: 1 }; value }; };
    counter.value
}
"#),
        1
    );
}

#[test]
fn used_loop_break_value_transfers_its_local_tail() {
    assert_eq!(
        run(r#"
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    {
        let value = loop { break { let inner = Watch { counter: &mut counter, digit: 1 }; inner }; };
        if counter.value != 0 { return 99; }
    }
    counter.value
}
"#),
        1
    );
}

#[test]
fn deferred_block_tail_is_discarded() {
    assert_eq!(
        run(r#"
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    { defer { let value = Watch { counter: &mut counter, digit: 1 }; value }; }
    counter.value
}
"#),
        1
    );
}

#[test]
fn finally_discards_its_tail_while_the_try_result_stays_live() {
    assert_eq!(
        run(r#"
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    {
        let value = try { let value = Watch { counter: &mut counter, digit: 1 }; value }
            finally { let cleanup = Watch { counter: &mut counter, digit: 2 }; cleanup };
        if counter.value != 2 { return 99; }
    }
    counter.value
}
"#),
        21
    );
}

#[test]
fn discarded_try_and_recover_tails_keep_cleanup() {
    for expr in [
        "try { let value = Watch { counter: &mut counter, digit: 1 }; value } recover { _ => { let value = Watch { counter: &mut counter, digit: 2 }; value } }",
        "try { throw 7; } recover { _ => { let value = Watch { counter: &mut counter, digit: 1 }; value } }",
    ] {
        let source = format!(
            "fn probe() -> Int {{ let mut counter = Count {{ value: 0 }}; {expr}; counter.value }}"
        );
        assert_eq!(run(&source), 1, "{expr}");
    }
}

#[test]
fn discarded_parent_still_consumes_condition_and_call_argument_values() {
    assert_eq!(
        run(r#"
fn inspect(counter: &Count) -> Bool { counter.value == 0 }
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    if inspect({ let alias = &counter; alias }) {
        let value = Watch { counter: &mut counter, digit: 1 }; value
    } else { let value = Watch { counter: &mut counter, digit: 2 }; value };
    counter.value
}
"#),
        1
    );
}
