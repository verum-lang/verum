//! The operator precedence ladder, pinned by SHAPE (T0816).
//!
//! Every row states how an expression is ACTUALLY grouped, not merely
//! that it parses.  That distinction is the whole point of this file:
//! `precedence_tests.rs` carried 99 tests, 96 of which only called
//! `assert_parses`, so `test_bitwise_and_before_or` passed no matter
//! which way `&` and `|` bound — and `grammar/verum.ebnf` drifted to a
//! single flat bitwise level while the parser used the C ladder, giving
//! the two documents different VALUES for `a | b & c`.
//!
//! The ladder these rows encode, LOOSEST first:
//!
//! ```text
//!   |>                                   left
//!   =  +=  -=  *=  /=  %=  &=  |=  ^=  <<=  >>=      right
//!   ??   ..  ..=   ->  implies  =>  <->  iff          right / range
//!   ||                                   left
//!   &&                                   left
//!   ==  !=  <  <=  >  >=  in  is         left  (ONE level)
//!   |                                    left
//!   ^                                    left
//!   &                                    left
//!   <<  >>                               left
//!   +  -                                 left
//!   *  /  %                              left
//!   **                                   right
//!   as                                   left
//!   prefix  !  -  ~  &  *
//!   postfix  .f  .m()  ?.  []  ()  ?  .await
//! ```
//!
//! Three sources have to agree on it: this file, `grammar/verum.ebnf`
//! (§2.10) and the published operator table
//! (`internal/website/docs/reference/operators.md`).  Change one, change
//! all three — a row here failing means the parser moved.

use verum_ast::expr::{Expr, ExprKind};
use verum_common::FileId;
use verum_fast_parser::VerumParser;

fn parse(source: &str) -> Result<Expr, String> {
    let parser = VerumParser::new();
    parser
        .parse_expr_str(source, FileId::new(0))
        .map_err(|e| format!("{e:?}"))
}

/// Render an expression as `(op left right)` — leaves as their own text.
fn sexp(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Binary { op, left, right } => {
            format!("({:?} {} {})", op, sexp(left), sexp(right))
        }
        ExprKind::Unary { op, expr } => format!("({:?} {})", op, sexp(expr)),
        ExprKind::Pipeline { left, right } => format!("(|> {} {})", sexp(left), sexp(right)),
        ExprKind::NullCoalesce { left, right } => format!("(?? {} {})", sexp(left), sexp(right)),
        ExprKind::Cast { expr, .. } => format!("(as {} T)", sexp(expr)),
        ExprKind::Range {
            start,
            end,
            inclusive,
        } => {
            let s = start.as_ref().map(|x| sexp(x)).unwrap_or_default();
            let t = end.as_ref().map(|x| sexp(x)).unwrap_or_default();
            format!("({} {} {})", if *inclusive { "..=" } else { ".." }, s, t)
        }
        ExprKind::Is { expr, negated, .. } => format!(
            "({} {} pat)",
            if *negated { "is-not" } else { "is" },
            sexp(expr)
        ),
        ExprKind::Paren(inner) => format!("(paren {})", sexp(inner)),
        ExprKind::Path(p) => match p.segments.last() {
            Some(verum_ast::PathSegment::Name(ident)) => ident.name.to_string(),
            _ => "?path".to_string(),
        },
        ExprKind::Literal(l) => format!("{:?}", l.kind)
            .replace("Int(IntLit { value: ", "")
            .chars()
            .take_while(|c| c.is_ascii_digit() || c.is_ascii_alphabetic())
            .collect::<String>(),
        ExprKind::MethodCall {
            receiver, method, ..
        } => format!("(. {} {})", sexp(receiver), method.name),
        ExprKind::Call { func, .. } => format!("(call {})", sexp(func)),
        ExprKind::Field { expr, field } => format!("(. {} {})", sexp(expr), field.name),
        ExprKind::Index { expr, index } => format!("([] {} {})", sexp(expr), sexp(index)),
        _ => "<other>".to_string(),
    }
}

/// `(source, how it groups)`.
const LADDER: &[(&str, &str)] = &[
    // --- bitwise: the C ladder, & tighter than ^ tighter than | -------
    ("a | b & c", "(BitOr a (BitAnd b c))"),
    ("a ^ b & c", "(BitXor a (BitAnd b c))"),
    ("a & b | c", "(BitOr (BitAnd a b) c)"),
    ("d | a ^ b", "(BitOr d (BitXor a b))"),
    ("a | b ^ c & d", "(BitOr a (BitXor b (BitAnd c d)))"),
    ("a << 1 | c", "(BitOr (Shl a 1) c)"),
    // --- comparison is ONE left-associative level ---------------------
    ("a == b < c", "(Lt (Eq a b) c)"),
    ("a < b == c", "(Eq (Lt a b) c)"),
    ("a != b >= c", "(Ge (Ne a b) c)"),
    ("a < b < c", "(Lt (Lt a b) c)"),
    // --- bitwise binds tighter than comparison ------------------------
    ("a & b == c", "(Eq (BitAnd a b) c)"),
    ("a == b & c", "(Eq a (BitAnd b c))"),
    // --- logical ------------------------------------------------------
    ("a || b && c", "(Or a (And b c))"),
    ("a && b == c", "(And a (Eq b c))"),
    // --- null coalescing: right-associative, looser than || -----------
    ("a ?? b ?? c", "(?? a (?? b c))"),
    ("a ?? b || c", "(?? a (Or b c))"),
    ("a ?? b + c", "(?? a (Add b c))"),
    // --- range --------------------------------------------------------
    ("0 .. n - 1", "(.. 0 (Sub n 1))"),
    ("a .. b == c", "(.. a (Eq b c))"),
    ("a < b .. c", "(.. (Lt a b) c)"),
    ("0 ..= n - 1", "(..= 0 (Sub n 1))"),
    ("a ..= b && c", "(..= a (And b c))"),
    // --- cast: looser than a prefix op, tighter than arithmetic -------
    ("a as Int + b", "(Add (as a T) b)"),
    ("a + b as Int", "(Add a (as b T))"),
    ("a as Int * b", "(Mul (as a T) b)"),
    ("-a as Int", "(as (Neg a) T)"),
    ("!a as Bool", "(as (Not a) T)"),
    ("a as Int as Float", "(as (as a T) T)"),
    // --- shift vs arithmetic ------------------------------------------
    ("a + b << c", "(Shl (Add a b) c)"),
    ("a << b + c", "(Shl a (Add b c))"),
    // --- power: right-associative, looser than a prefix op ------------
    ("2 ** 3 ** 4", "(Pow 2 (Pow 3 4))"),
    ("-2 ** 2", "(Pow (Neg 2) 2)"),
    ("2 * 3 ** 2", "(Mul 2 (Pow 3 2))"),
    // --- implication: right-associative, looser than || ---------------
    ("a implies b implies c", "(Imply a (Imply b c))"),
    ("a || b implies c", "(Imply (Or a b) c)"),
    ("a -> b", "(Imply a b)"),
    ("a <-> b", "(Iff a b)"),
    ("a <-> b implies c", "(Iff a (Imply b c))"),
    ("a implies b == c", "(Imply a (Eq b c))"),
    ("a implies b .. c", "(Imply a (.. b c))"),
    ("a .. b implies c", "(Imply (.. a b) c)"),
    // --- containment and pattern test sit on the comparison level -----
    ("a in b == c", "(Eq (In a b) c)"),
    ("a in b && c", "(And (In a b) c)"),
    ("a is Some(v) && b", "(And (is a pat) b)"),
    ("a == b is Some(v)", "(is (Eq a b) pat)"),
    // --- assignment: looser than everything but the pipeline ----------
    //
    // `x = a ?? b` grouping as `(x = a) ?? b` was the T0816 defect:
    // the parser put assignment ABOVE `??`, against both the grammar
    // and the published table.
    ("x = a ?? b", "(Assign x (?? a b))"),
    ("x = a || b", "(Assign x (Or a b))"),
    ("x = a implies b", "(Assign x (Imply a b))"),
    ("x = a .. b", "(Assign x (.. a b))"),
    ("x = y = z", "(Assign x (Assign y z))"),
    ("x += a + b", "(AddAssign x (Add a b))"),
    // --- pipeline is the loosest operator there is --------------------
    ("x = a |> f", "(|> (Assign x a) f)"),
    ("a ?? b |> f", "(|> (?? a b) f)"),
    ("a |> f ?? b", "(|> a (?? f b))"),
    // --- claims that lived only in comments ---------------------------
    //
    // Eight groupings were stated in `grammar_tests.rs` comments beside
    // a body that only called `assert_expr_parses`, so the claim was
    // never compared against anything.  Checked here instead.
    ("x + 1 |> f", "(|> (Add x 1) f)"),
    ("a && b || c && d", "(Or (And a b) (And c d))"),
    ("a + b == c + d", "(Eq (Add a b) (Add c d))"),
    ("a + b << 2", "(Shl (Add a b) 2)"),
    ("a + b * c", "(Add a (Mul b c))"),
    ("a * b ** c", "(Mul a (Pow b c))"),

    // --- parentheses survive into the AST -----------------------------
    // The pretty-printer relies on this: it prints binary operands with
    // no parentheses of its own, so a dropped `Paren` node would silently
    // reassociate the user's expression on `verum fmt`.
    ("(a | b) & c", "(BitAnd (paren (BitOr a b)) c)"),
    ("(a + b) * c", "(Mul (paren (Add a b)) c)"),
    ("a * (b + c)", "(Mul a (paren (Add b c)))"),
];

#[test]
fn operator_ladder_groups_as_documented() {
    let mut drifted = Vec::new();
    for (src, expected) in LADDER {
        match parse(src) {
            Ok(e) => {
                let actual = sexp(&e);
                if actual != *expected {
                    drifted.push(format!("  {src:<22} expected {expected}\n  {:<22} actual   {actual}", ""));
                }
            }
            Err(err) => drifted.push(format!(
                "  {src:<22} expected {expected}\n  {:<22} actual   PARSE ERROR: {}",
                "",
                err.chars().take(80).collect::<String>()
            )),
        }
    }
    assert!(
        drifted.is_empty(),
        "the operator ladder moved — {} of {} rows differ.\n\
         Update grammar/verum.ebnf §2.10 and the website operator table \
         together with this file:\n{}",
        drifted.len(),
        LADDER.len(),
        drifted.join("\n")
    );
}

/// A row that cannot silently pass: if `sexp` ever stopped distinguishing
/// groupings, every row above would compare equal to itself and the gate
/// would be vacuous.  This pins that the renderer DOES separate the two
/// readings of the one expression the whole task turns on.
#[test]
fn the_gate_can_tell_the_two_readings_apart() {
    let c_ladder = sexp(&parse("a | b & c").expect("parses"));
    let flat = sexp(&parse("(a | b) & c").expect("parses"));
    assert_ne!(
        c_ladder, flat,
        "the renderer cannot distinguish `a | b & c` from `(a | b) & c`, \
         so every row in this file would pass regardless of precedence"
    );
}
