//! Parser-surface regression tests for the `core.proof.tactics`
//! 7-file stdlib layout.
//!
//! Each test parses a representative tactic declaration from the
//! stdlib's seven files and asserts the parser accepts the shape.
//! If a parser change breaks one of the stdlib's tactic syntaxes,
//! these tests catch it before the stdlib itself fails to compile.

use verum_ast::FileId;
use verum_fast_parser::RecursiveParser;
use verum_lexer::Lexer;

fn parse_ok(content: &str) -> Result<(), String> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer
        .tokenize()
        .map_err(|e| format!("LEXER ERROR: {:?}", e))?;
    let mut parser = RecursiveParser::new(&tokens, file_id);
    parser
        .parse_module()
        .map(|_| ())
        .map_err(|e| format!("PARSE ERROR: {:?}", e))
}

// -- core.proof.tactics.basic --------------------------------------

#[test]
fn tactic_refl_zero_arg() {
    // `tactic refl() { refl }` — the smallest shape.
    parse_ok("tactic refl() { refl }").unwrap();
}

#[test]
fn tactic_trivial_with_first_combinator() {
    parse_ok("tactic trivial() { first { refl; assumption; simp } }")
        .unwrap();
}

#[test]
fn tactic_exact_with_expr_param() {
    parse_ok("tactic exact(term: Expr) { exact(term) }").unwrap();
}

#[test]
fn tactic_by_axiom_with_text_param() {
    parse_ok("tactic by_axiom(name: Text) { by_axiom(name) }").unwrap();
}

// -- core.proof.tactics.logical ------------------------------------

#[test]
fn tactic_intro_as_with_text_arg() {
    parse_ok("tactic intro_as(name: Text) { intro(name) }").unwrap();
}

#[test]
fn tactic_witness_with_expr_arg() {
    parse_ok("tactic witness(term: Expr) { witness(term) }").unwrap();
}

// -- core.proof.tactics.structural ---------------------------------

#[test]
fn tactic_induction_with_hypothesis_param() {
    parse_ok("tactic induction(var: Hypothesis) { induction(var) }")
        .unwrap();
}

#[test]
fn tactic_destruct_as_with_list_param() {
    parse_ok(
        "tactic destruct_as(var: Hypothesis, names: List<Text>) \
         { destruct_as(var, names) }",
    )
    .unwrap();
}

// -- core.proof.tactics.rewrite ------------------------------------

#[test]
fn tactic_rewrite_with_expr() {
    parse_ok("tactic rewrite(eq: Expr) { rewrite(eq) }").unwrap();
}

#[test]
fn tactic_simp_with_lemmas() {
    parse_ok(
        "tactic simp_with(lemmas: List<Expr>) { simp_with(lemmas) }",
    )
    .unwrap();
}

// -- core.proof.tactics.combinators --------------------------------

#[test]
fn tactic_seq_with_two_tactic_params() {
    parse_ok("tactic seq(first: Tactic, then: Tactic) { first; then }")
        .unwrap();
}

#[test]
fn tactic_orelse_with_try_else_body() {
    parse_ok(
        "tactic orelse(primary: Tactic, fallback: Tactic) {
            try { primary } else { fallback }
        }",
    )
    .unwrap();
}

#[test]
fn tactic_repeat_n_with_int_param() {
    parse_ok(
        "tactic repeat_n(count: Int, body: Tactic) { \
         repeat(count) { body } }",
    )
    .unwrap();
}

#[test]
fn tactic_bare_fail_no_message() {
    // `fail` without a message — identity element for OrElse.
    // Stdlib `core.proof.tactics.combinators` at line 27.
    parse_ok("tactic fail() { fail }").unwrap();
}

#[test]
fn tactic_first_of_with_list_arg() {
    // `first(expr)` — dynamic form dispatched through the generic
    // Named-tactic path; needed by `tactic first_of(alternatives:
    // List<Tactic>) { first(alternatives) }`.
    parse_ok("tactic first_of(alternatives: List<Tactic>) { first(alternatives) }")
        .unwrap();
}

// -- core.proof.tactics.meta ---------------------------------------

#[test]
fn tactic_quote_with_tactic_param() {
    parse_ok("tactic quote(body: Tactic) { quote { body } }").unwrap();
}

#[test]
fn tactic_goal_intro_zero_arg() {
    parse_ok("tactic goal_intro() { goal_intro }").unwrap();
}

// -- Multi-tactic module -------------------------------------------

#[test]
fn multiple_tactics_in_one_file() {
    parse_ok(
        "tactic refl() { refl }
         tactic assumption() { assumption }
         tactic trivial() { first { refl; assumption } }",
    )
    .unwrap();
}

// -- Tactic with `public` visibility -------------------------------

#[test]
fn public_tactic_decl() {
    parse_ok("public tactic refl() { refl }").unwrap();
}
