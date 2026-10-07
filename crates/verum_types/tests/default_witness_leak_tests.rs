//! T1629 regressions for the former Default witness leak (A64).
//!
//! Builtin Default.default used the globally live TypeVar(0) as Self. The
//! first generic function could accidentally share that identity; later
//! functions or implement methods could instead inherit an unrelated free
//! variable. These retain the original method-order, protocol, match, deref,
//! annotation and generic-payload controls, now asserting the intended result.
//! All error diagnostics are collected: absence of E404 alone is not success.
//! Fresh-process allocation controls live in default_witness_identity.rs.
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

fn errors(code: &str) -> List<Text> {
    let module = Parser::new(code)
        .parse_module()
        .expect("parse should succeed");
    let mut checker = TypeChecker::new();
    let mut errors = List::new();
    for item in &module.items {
        if let verum_ast::ItemKind::Type(declaration) = &item.kind {
            if let Err(error) = checker.register_type_declaration(declaration) {
                errors.push(Text::from(format!("{error:?}")));
            }
        }
    }
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(Text::from(format!("{error:?}")));
        }
    }
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| Text::from(format!("{d:?}"))),
    );
    errors
}

fn assert_clean(source: &str) {
    let errors = errors(source);
    assert!(errors.is_empty(), "all errors: {errors:?}\n{source}");
}

// Zero and One are source declarations here; this isolated checker has no
// stdlib. An unresolved protocol must not masquerade as a clean control.
const DECL: &str = r#"
type Zero is protocol { fn zero()->Self; };
type One is protocol { fn one()->Self; };
type M<T> is None | Some(T);
"#;
const TAKE: &str = r#"
    public fn take(&mut self)->M<T> {
        let old = *self;
        *self = None;
        old
    }
"#;

fn impl_block(first: &str, second: &str) -> Text {
    Text::from(format!("{DECL}implement<T> M<T> {{\n{first}\n{second}}}\n"))
}

fn call_method(call: &str, bound: &str) -> Text {
    Text::from(format!(
        "public fn d(self)->T where T: {bound} {{ {call} }}"
    ))
}

#[test]
fn default_and_take_are_independent_of_method_order() {
    let default = call_method("T.default()", "Default");
    assert_clean(&impl_block(&default, TAKE));
    assert_clean(&impl_block(TAKE, &default));
}

#[test]
fn default_zero_and_one_preserve_the_same_self_identity() {
    for (call, bound) in [
        ("T.default()", "Default"),
        ("T.zero()", "Zero"),
        ("T.one()", "One"),
    ] {
        assert_clean(&impl_block(&call_method(call, bound), TAKE));
    }
}

#[test]
fn a_match_on_self_without_default_does_not_leak() {
    let first = r#"
        public fn d(self)->M<T> {
            match self { Some(v) => M.Some(v), None => M.None }
        }
    "#;
    assert_clean(&impl_block(first, TAKE));
}

#[test]
fn a_deref_of_a_mut_variant_receiver_is_fine_alone() {
    assert_clean(&impl_block("", TAKE));
}

#[test]
fn a_let_bound_self_after_a_default_call_retains_its_type() {
    let binding = "public fn take(&mut self)->Int { let r = self; 1 }";
    assert_clean(&impl_block(&call_method("T.default()", "Default"), binding));
    assert_clean(&impl_block(
        &call_method("T.default()", "Default"),
        "public fn take(&mut self)->Int { let x = 1; x }",
    ));
    assert_clean(&impl_block(&call_method("T.zero()", "Zero"), binding));
    assert_clean(&impl_block("", binding));
}

#[test]
fn later_lets_keep_the_implement_parameter_identity() {
    for second in [
        "public fn f(&mut self)->Int { let r = self; 1 }",
        "public fn f(&mut self)->Int { let r: M<T> = None; 1 }",
        "public fn f(&mut self, m: M<T>)->Int { let r = m; 1 }",
        "public fn f(&mut self)->Int { let r: M<Int> = None; 1 }",
        "public fn f(&mut self)->M<T> { None }",
        "public fn f(&mut self)->M<T> { let old: M<T> = *self; old }",
    ] {
        assert_clean(&impl_block(&call_method("T.default()", "Default"), second));
    }
}

#[test]
fn a_let_bound_default_call_retains_the_free_function_parameter() {
    assert_clean("fn d<T>()->Int where T: Default { let x = T.default(); 1 }");
    assert_clean("fn d<T>()->Int where T: Default { let x: T = T.default(); 1 }");
}
