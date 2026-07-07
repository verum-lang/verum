//! Fixture corpus: the agreement surface plus pinned known divergences.
//!

//! Every fixture is one Verum function (correct Verum syntax — see
//! `grammar/verum.ebnf`) plus a fixed argument list, both representable in
//! the two engines' value models. Fixtures with `Expectation::Agree` form
//! the convergence contract; `Expectation::Pinned` fixtures assert the
//! *current shape* of a known divergence so that drift or disappearance
//! fires visibly.
//!

//! Divergence mechanics referenced below (verified against the source, not
//! folklore):
//!

//! * Tree-walk integer arithmetic is raw `i128` (`value_ops.rs`:
//!   `MetaValue::Int(a + b)`), so an `i128` overflow **panics the engine**
//!   under `overflow-checks` (dev profile). VBC integer arithmetic is
//!   `i64` `wrapping_add`/`wrapping_mul`
//!   (`interpreter/dispatch_table/handlers/integer_arith.rs`).
//! * Tree-walk `Int + Float` is a type-mismatch error (no coercion);
//!   VBC codegen coerces numeric operands.
//! * Tree-walk `&&` / `||` evaluate **both** operands
//!   (`MetaExpr::Binary` eval is eager, then boolean fold); VBC compiles
//!   short-circuit control flow.
//! * VBC's value model stores chars as `i64` codepoints; the tree-walk
//!   keeps `ConstValue::Char`.
//! * The tree-walk's Text method surface is a small subset (`len`,
//!   `to_uppercase`, `to_lowercase`, `trim`, `contains`, `starts_with`, …)
//!   and its float formatting is Rust `{}` (`5`), while VBC formats floats
//!   through `float_to_text` (`5.0`).

use crate::compare::{OutcomeKind, Verdict};
use crate::engines::Arg;

/// What the harness expects from one fixture run.
#[derive(Debug, Clone)]
pub enum Expectation {
    /// Both engines must agree (shape and value).
    Agree,
    /// A known divergence: the verdict must match `shape`. Anything else —
    /// including agreement — fires the pin.
    Pinned {
        /// The expected divergence shape.
        shape: PinShape,
        /// Why this divergence exists (source-verified mechanism).
        note: &'static str,
    },
}

/// Shape-matching for pinned divergences: strict enough that drift is
/// caught, loose enough that error wording / panic payloads can evolve.
#[derive(Debug, Clone, PartialEq)]
pub enum PinShape {
    /// Outcome shapes differ, e.g. one engine returns a value while the
    /// other errors or panics.
    Outcome {
        /// Expected VBC outcome shape.
        vbc: OutcomeKind,
        /// Expected tree-walk outcome shape.
        tree: OutcomeKind,
    },
    /// Both return values but of different comparable kinds.
    Type {
        /// Expected VBC comparable kind.
        vbc_kind: &'static str,
        /// Expected tree-walk comparable kind.
        tree_kind: &'static str,
    },
    /// Both return values of the same kind with different values.
    Value,
    /// Both return values the extractor deliberately refuses to decode.
    OpaqueBoth,
}

impl PinShape {
    /// Does `verdict` match this pin's shape?
    pub fn matches(&self, verdict: &Verdict) -> bool {
        match (self, verdict) {
            (
                PinShape::Outcome { vbc, tree },
                Verdict::OutcomeMismatch {
                    vbc: got_vbc,
                    tree: got_tree,
                    ..
                },
            ) => vbc == got_vbc && tree == got_tree,
            (
                PinShape::Type { vbc_kind, tree_kind },
                Verdict::TypeMismatch {
                    vbc_kind: got_vbc,
                    tree_kind: got_tree,
                    ..
                },
            ) => vbc_kind == got_vbc && tree_kind == got_tree,
            (PinShape::Value, Verdict::ValueMismatch { .. }) => true,
            (PinShape::OpaqueBoth, Verdict::OpaqueBoth { .. }) => true,
            _ => false,
        }
    }
}

impl std::fmt::Display for PinShape {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PinShape::Outcome { vbc, tree } => {
                write!(f, "outcome-mismatch(vbc={vbc}, tree={tree})")
            }
            PinShape::Type { vbc_kind, tree_kind } => {
                write!(f, "type-mismatch(vbc={vbc_kind}, tree={tree_kind})")
            }
            PinShape::Value => write!(f, "value-mismatch"),
            PinShape::OpaqueBoth => write!(f, "opaque-both"),
        }
    }
}

/// One differential fixture.
#[derive(Debug, Clone)]
pub struct Fixture {
    /// Unique fixture name.
    pub name: &'static str,
    /// One-line description for report output.
    pub description: &'static str,
    /// Verum source containing the fixture function.
    pub source: &'static str,
    /// The function to execute.
    pub fn_name: &'static str,
    /// Arguments (identical for both engines).
    pub args: Vec<Arg>,
    /// Expected relationship between the two engines.
    pub expect: Expectation,
}

/// Runtime probe: are integer `overflow-checks` active in this build?
///

/// The tree-walk's i128-overflow **panic** only exists when the harness
/// binary was compiled with `overflow-checks = true` (default dev profile);
/// under `--release` the same fixture wraps and the engines happen to agree.
/// The pin for that fixture is therefore selected per-profile — honestly —
/// instead of hardcoding one profile's behavior.
pub fn overflow_checks_enabled() -> bool {
    std::panic::catch_unwind(|| {
        let a: i128 = std::hint::black_box(i128::MAX);
        let b: i128 = std::hint::black_box(1);
        std::hint::black_box(a + b)
    })
    .is_err()
}

/// The full fixture corpus.
///

/// Call inside [`crate::engines::with_quiet_panics`] if panic noise from the
/// [`overflow_checks_enabled`] probe matters to the caller.
pub fn all_fixtures() -> Vec<Fixture> {
    let overflow_checks = overflow_checks_enabled();

    let mut fixtures = vec![
        // ================= agreement surface =================
        Fixture {
            name: "agree_add_small",
            description: "in-range integer addition",
            source: "fn add(a: Int, b: Int) -> Int { a + b }",
            fn_name: "add",
            args: vec![Arg::Int(3), Arg::Int(4)],
            expect: Expectation::Agree,
        },
        Fixture {
            name: "agree_arith_compound",
            description: "compound integer arithmetic with precedence (paren-free: see pin_paren_expr_quoted)",
            source: "fn mix(a: Int, b: Int) -> Int { a * b - 2 }",
            fn_name: "mix",
            args: vec![Arg::Int(5), Arg::Int(4)],
            expect: Expectation::Agree,
        },
        Fixture {
            name: "agree_rem",
            description: "integer remainder",
            source: "fn rem(a: Int, b: Int) -> Int { a % b }",
            fn_name: "rem",
            args: vec![Arg::Int(7), Arg::Int(3)],
            expect: Expectation::Agree,
        },
        Fixture {
            name: "agree_neg",
            description: "unary integer negation",
            source: "fn ng(a: Int) -> Int { -a }",
            fn_name: "ng",
            args: vec![Arg::Int(42)],
            expect: Expectation::Agree,
        },
        Fixture {
            name: "agree_div_exact",
            description: "integer division",
            source: "fn dv(a: Int, b: Int) -> Int { a / b }",
            fn_name: "dv",
            args: vec![Arg::Int(7), Arg::Int(2)],
            expect: Expectation::Agree,
        },
        Fixture {
            name: "agree_div_by_zero_errors",
            description: "integer division by zero errors in BOTH engines (shape-level)",
            source: "fn dvz(a: Int, b: Int) -> Int { a / b }",
            fn_name: "dvz",
            args: vec![Arg::Int(7), Arg::Int(0)],
            expect: Expectation::Agree,
        },
        Fixture {
            name: "agree_float_arith",
            description: "float multiply-add (epsilon-compared)",
            source: "fn fa(a: Float, b: Float) -> Float { a * b + 1.5 }",
            fn_name: "fa",
            args: vec![Arg::Float(2.0), Arg::Float(3.0)],
            expect: Expectation::Agree,
        },
        Fixture {
            name: "agree_bool_logic",
            description: "bool and/not with pure operands",
            source: "fn bl(a: Bool, b: Bool) -> Bool { a && !b }",
            fn_name: "bl",
            args: vec![Arg::Bool(true), Arg::Bool(false)],
            expect: Expectation::Agree,
        },
        Fixture {
            name: "agree_cmp_lt",
            description: "integer less-than",
            source: "fn lt(a: Int, b: Int) -> Bool { a < b }",
            fn_name: "lt",
            args: vec![Arg::Int(3), Arg::Int(4)],
            expect: Expectation::Agree,
        },
        Fixture {
            name: "agree_if_then",
            description: "if/else control flow (then branch)",
            source: "fn cf(n: Int) -> Int { if n > 10 { 1 } else { 2 } }",
            fn_name: "cf",
            args: vec![Arg::Int(11)],
            expect: Expectation::Agree,
        },
        Fixture {
            name: "agree_if_else",
            description: "if/else control flow (else branch)",
            source: "fn cf(n: Int) -> Int { if n > 10 { 1 } else { 2 } }",
            fn_name: "cf",
            args: vec![Arg::Int(2)],
            expect: Expectation::Agree,
        },
        Fixture {
            name: "agree_let_binding",
            description: "let binding + use",
            source: "fn lb(n: Int) -> Int { let x = n * 2; x + 1 }",
            fn_name: "lb",
            args: vec![Arg::Int(20)],
            expect: Expectation::Agree,
        },
        Fixture {
            name: "agree_text_concat",
            description: "Text literal concatenation (heap/small-string decode on VBC side)",
            source: "fn tc() -> Text { \"ab\" + \"cd\" }",
            fn_name: "tc",
            args: vec![],
            expect: Expectation::Agree,
        },
        Fixture {
            name: "agree_text_len",
            description: "Text overlap subset: len (tree-walk UInt folds into Int domain)",
            source: "fn tl() -> Int { \"hello\".len() }",
            fn_name: "tl",
            args: vec![],
            expect: Expectation::Agree,
        },
        // ================= pinned known divergences =================
        Fixture {
            name: "pin_i64_wrap_vs_i128",
            description: "i64::MAX + 1: VBC wraps at i64, tree-walk keeps exact i128",
            source: "fn add(a: Int, b: Int) -> Int { a + b }",
            fn_name: "add",
            args: vec![Arg::Int(i64::MAX), Arg::Int(1)],
            expect: Expectation::Pinned {
                shape: PinShape::Value,
                note: "VBC AddI is i64 wrapping_add -> i64::MIN; tree-walk value_ops adds \
                       in i128 -> 9223372036854775808 (no wrap). Same Int kind, different value.",
            },
        },
        Fixture {
            name: "pin_int_float_mixed",
            description: "Int + Float: VBC PANICS (float-op on int operand), tree-walk errors cleanly",
            source: "fn mx(a: Int, b: Float) -> Float { a + b }",
            fn_name: "mx",
            args: vec![Arg::Int(1), Arg::Float(2.5)],
            expect: Expectation::Pinned {
                shape: PinShape::Outcome {
                    vbc: OutcomeKind::Panic,
                    tree: OutcomeKind::Error,
                },
                note: "both engines reject mixed Int+Float, in different shapes: VBC codegen \
                       emits a float add whose int operand trips an interpreter panic \
                       ('Expected float, got Some(1)') — a compiler-process crash, not a \
                       diagnostic; tree-walk MetaValueOps::add has no (Int, Float) arm and \
                       returns 'Type mismatch in addition'. Observed 2026-07-07.",
            },
        },
        Fixture {
            name: "pin_no_short_circuit_and",
            description: "a > 0 && 10/a > 1 at a=0: VBC short-circuits, tree-walk eagerly divides by zero",
            // Deliberately paren-free: a parenthesized RHS would hit the
            // pin_paren_expr_quoted gap first and mask the eager-eval one.
            source: "fn sc(a: Int) -> Bool { a > 0 && 10 / a > 1 }",
            fn_name: "sc",
            args: vec![Arg::Int(0)],
            expect: Expectation::Pinned {
                shape: PinShape::Outcome {
                    vbc: OutcomeKind::Value,
                    tree: OutcomeKind::Error,
                },
                note: "tree-walk MetaExpr::Binary evaluates BOTH operands before the And fold -> \
                       DivisionByZero; VBC emits short-circuit branches -> Bool(false).",
            },
        },
        Fixture {
            name: "pin_paren_expr_quoted",
            description: "(a + b) * 2: tree-walk QUOTES parenthesized exprs instead of evaluating them",
            source: "fn pq(a: Int, b: Int) -> Int { (a + b) * 2 }",
            fn_name: "pq",
            args: vec![Arg::Int(3), Arg::Int(4)],
            expect: Expectation::Pinned {
                shape: PinShape::Outcome {
                    vbc: OutcomeKind::Value,
                    tree: OutcomeKind::Error,
                },
                note: "ast_expr_to_meta_expr has no ExprKind::Paren arm -> the fallback returns \
                       MetaExpr::Quote(expr) -> ConstValue::Expr -> 'Type mismatch in \
                       multiplication'; VBC evaluates parens normally (Int(14)). \
                       Harness discovery, 2026-07-07.",
            },
        },
        Fixture {
            name: "pin_fstring_float_format",
            description: "f\"{x}\" of a Float: VBC formats \"5.0\", tree-walk quotes the f-string as AST",
            source: "fn ff(x: Float) -> Text { f\"{x}\" }",
            fn_name: "ff",
            args: vec![Arg::Float(5.0)],
            expect: Expectation::Pinned {
                shape: PinShape::Type {
                    vbc_kind: "Text",
                    tree_kind: "Opaque",
                },
                note: "VBC compiles the f-string and float_to_text yields \"5.0\"; the \
                       tree-walk has no ExprKind::InterpolatedString arm, so the same \
                       quote-fallback as pin_paren_expr_quoted returns the f-string as an \
                       AST value (Opaque[tree-ast-expr]). The '5'-vs-'5.0' float-format \
                       divergence (Rust {} in evaluator builtins vs float_to_text) is \
                       therefore unreachable via f-strings today; this pin guards the \
                       deeper gap. Observed 2026-07-07.",
            },
        },
        Fixture {
            name: "pin_char_codepoint",
            description: "Char literal: VBC value model stores i64 codepoint, tree-walk keeps Char",
            source: "fn ch() -> Char { 'A' }",
            fn_name: "ch",
            args: vec![],
            expect: Expectation::Pinned {
                shape: PinShape::Type {
                    vbc_kind: "Int",
                    tree_kind: "Char",
                },
                note: "VBC NaN-boxing has no Char tag (codepoint as Int 65); \
                       tree-walk ConstValue::Char('A').",
            },
        },
        Fixture {
            name: "pin_collections_opaque",
            description: "List literal: both engines return collections the extractor refuses to decode",
            source: "fn ar() -> List<Int> { [1, 2, 3] }",
            fn_name: "ar",
            args: vec![],
            expect: Expectation::Pinned {
                shape: PinShape::OpaqueBoth,
                note: "extractor contract: collections are Opaque (never mis-compared); \
                       structural decode is step (ii) with script_engine::extract_owned as donor.",
            },
        },
    ];

    // Profile-dependent pin: the tree-walk's raw i128 arithmetic. With
    // overflow-checks ON (dev profile) the multiply panics the engine; with
    // them OFF (release) both engines wrap to the same 0 and agree.
    fixtures.push(Fixture {
        name: "pin_i128_overflow_panic",
        description: "(2^62 * 2^62) * 16 overflows i128: tree-walk PANICS under overflow-checks",
        source: "fn ovm(a: Int, b: Int) -> Int { a * a * b }",
        fn_name: "ovm",
        args: vec![Arg::Int(1_i64 << 62), Arg::Int(16)],
        expect: if overflow_checks {
            Expectation::Pinned {
                shape: PinShape::Outcome {
                    vbc: OutcomeKind::Value,
                    tree: OutcomeKind::Panic,
                },
                note: "tree-walk value_ops MetaValue::Int(a * b) is raw i128 arithmetic -> \
                       'attempt to multiply with overflow' panic (observed via catch_unwind); \
                       VBC wraps at i64 (2^62*2^62 mod 2^64 = 0; 0*16 = 0).",
            }
        } else {
            // overflow-checks off: i128 wraps to 0, i64 wraps to 0 — agreement.
            Expectation::Agree
        },
    });

    fixtures
}
