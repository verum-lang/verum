//! Spec: Verum Grammar §let_stmt requires a terminating semicolon.
//! Line breaks do not insert one. These active checks replace three ignored
//! tests of an unadopted automatic-insertion design (T1500).

use verum_ast::FileId;
use verum_fast_parser::VerumParser;

fn assert_missing_semicolon(source: &str, count: usize) {
    let errors = VerumParser::new()
        .parse_module_str(source, FileId::new(0))
        .expect_err("a missing let terminator must be diagnosed");
    assert_eq!(errors.len(), count, "{errors:?}");
    assert!(
        errors.iter().all(|error| error.error_code() == "E010"),
        "{errors:?}"
    );
}

#[test]
fn missing_let_semicolons_in_a_loop_are_diagnosed() {
    assert_missing_semicolon(
        r#"
fn test() {
    let mut a = 0
    let mut b = 1
    while a <= b {
        let temp = a + b
        a = b;
        b = temp;
    }
    for i in 0..10 { print(i); }
}
"#,
        3,
    );
}

#[test]
fn an_explicit_semicolon_does_not_terminate_the_next_let() {
    assert_missing_semicolon(
        r#"
fn test() {
    let a = 1;
    let b = 2
    let c = 3;
}
"#,
        1,
    );
}

#[test]
fn a_line_break_before_if_does_not_terminate_a_let() {
    assert_missing_semicolon(
        r#"
fn test() {
    let x = 5
    if x > 3 {
        let y = x + 1;
    } else {
        let y = x - 1;
    }
}
"#,
        1,
    );
}

#[test]
fn explicit_semicolons_in_loops_and_branches_parse() {
    VerumParser::new()
        .parse_module_str(
            r#"
fn test() {
    let mut a = 0;
    let mut b = 1;
    while a <= b {
        let temp = a + b;
        a = b;
        b = temp;
    }
    if a > b { let x = a; } else { let x = b; }
}
"#,
            FileId::new(0),
        )
        .expect("explicit let terminators must parse");
}
