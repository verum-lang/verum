//! Outcome model and the vendored comparator.
//!

//! The comparator is deliberately **vendored** (a few dozen lines) instead
//! of linking the dormant, non-workspace-member `vcs/differential/runner`
//! crate — see the crate docs. Rules:
//!

//! * `Int` / `Bool` / `Text` / `Char` / `Unit`: exact equality.
//! * `Float`: relative-epsilon equality ([`float_eq`]) with `NaN == NaN`
//!   treated as agreement (both engines produced "not a number"; bitwise
//!   NaN payloads are not semantics).
//! * `Opaque`: **never** equal to anything — two opaque results yield
//!   [`Verdict::OpaqueBoth`] (counted separately, not as agreement), an
//!   opaque paired with a scalar is a [`Verdict::TypeMismatch`].
//! * `Error` vs `Error`: shape-level agreement. The two engines have
//!   disjoint error taxonomies (`MetaError` vs `VbcExecutionError` /
//!   interpreter traps), so comparing message text would pin wording, not
//!   semantics.
//! * `Panic` anywhere is **never** agreement: a panic is an engine defect
//!   by definition (the harness exists to observe them). Two panics yield
//!   [`Verdict::BothPanicked`].

use crate::extractor::Comparable;

/// Relative epsilon for float comparison.
pub const FLOAT_EPSILON: f64 = 1e-9;

/// Vendored float comparator: exact match, or relative-epsilon closeness,
/// with `NaN == NaN` and same-signed infinities equal.
pub fn float_eq(a: f64, b: f64) -> bool {
    if a == b {
        return true; // covers equal finites and same-signed infinities
    }
    if a.is_nan() && b.is_nan() {
        return true;
    }
    if a.is_infinite() || b.is_infinite() {
        return false; // one infinite, one not (or opposite signs)
    }
    let scale = 1.0_f64.max(a.abs()).max(b.abs());
    (a - b).abs() <= FLOAT_EPSILON * scale
}

/// What one engine produced for one fixture.
#[derive(Debug, Clone)]
pub enum EngineOutcome {
    /// The engine returned a value (already folded into the comparable domain).
    Value(Comparable),
    /// The engine returned an error through its own error channel
    /// (`MetaError` for the tree-walk, `VbcExecutionError` for VBC).
    Error(String),
    /// The engine panicked; caught by `catch_unwind`, payload preserved.
    Panic(String),
}

impl EngineOutcome {
    /// The outcome's shape, for pin matching.
    pub fn kind(&self) -> OutcomeKind {
        match self {
            EngineOutcome::Value(_) => OutcomeKind::Value,
            EngineOutcome::Error(_) => OutcomeKind::Error,
            EngineOutcome::Panic(_) => OutcomeKind::Panic,
        }
    }
}

impl std::fmt::Display for EngineOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineOutcome::Value(v) => write!(f, "Value({v})"),
            EngineOutcome::Error(e) => write!(f, "Error({e})"),
            EngineOutcome::Panic(p) => write!(f, "Panic({p})"),
        }
    }
}

/// Outcome shape (value vs error vs panic) — the coarsest divergence axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeKind {
    /// Engine returned a value.
    Value,
    /// Engine returned an error.
    Error,
    /// Engine panicked.
    Panic,
}

impl std::fmt::Display for OutcomeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutcomeKind::Value => write!(f, "Value"),
            OutcomeKind::Error => write!(f, "Error"),
            OutcomeKind::Panic => write!(f, "Panic"),
        }
    }
}

/// The classified relationship between the two engines' outcomes.
#[derive(Debug, Clone)]
pub enum Verdict {
    /// Same shape, same (comparable) value.
    Agree,
    /// Both engines returned values the extractor cannot faithfully decode.
    /// Deliberately *not* agreement — see the extractor contract.
    OpaqueBoth {
        /// Opaque kind label from the VBC side.
        vbc: String,
        /// Opaque kind label from the tree-walk side.
        tree: String,
    },
    /// The outcome *shapes* differ (e.g. one engine returned a value, the
    /// other errored or panicked).
    OutcomeMismatch {
        /// VBC outcome shape.
        vbc: OutcomeKind,
        /// Tree-walk outcome shape.
        tree: OutcomeKind,
        /// Human-readable rendering of both outcomes.
        detail: String,
    },
    /// Both returned values, but of different comparable kinds
    /// (also covers scalar-vs-opaque pairs).
    TypeMismatch {
        /// Comparable kind on the VBC side.
        vbc_kind: &'static str,
        /// Comparable kind on the tree-walk side.
        tree_kind: &'static str,
        /// Human-readable rendering of both values.
        detail: String,
    },
    /// Both returned values of the same kind, but unequal.
    ValueMismatch {
        /// Human-readable rendering of both values.
        detail: String,
    },
    /// Both engines panicked — never agreement (two defects, possibly
    /// different ones).
    BothPanicked {
        /// Human-readable rendering of both panic payloads.
        detail: String,
    },
}

impl Verdict {
    /// True only for [`Verdict::Agree`].
    pub fn is_agree(&self) -> bool {
        matches!(self, Verdict::Agree)
    }

    /// Short label for report lines.
    pub fn label(&self) -> &'static str {
        match self {
            Verdict::Agree => "agree",
            Verdict::OpaqueBoth { .. } => "opaque-both",
            Verdict::OutcomeMismatch { .. } => "outcome-mismatch",
            Verdict::TypeMismatch { .. } => "type-mismatch",
            Verdict::ValueMismatch { .. } => "value-mismatch",
            Verdict::BothPanicked { .. } => "both-panicked",
        }
    }
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Verdict::Agree => write!(f, "agree"),
            Verdict::OpaqueBoth { vbc, tree } => {
                write!(f, "opaque-both (vbc={vbc}, tree={tree})")
            }
            Verdict::OutcomeMismatch { vbc, tree, detail } => {
                write!(f, "outcome-mismatch (vbc={vbc}, tree={tree}): {detail}")
            }
            Verdict::TypeMismatch {
                vbc_kind,
                tree_kind,
                detail,
            } => write!(
                f,
                "type-mismatch (vbc={vbc_kind}, tree={tree_kind}): {detail}"
            ),
            Verdict::ValueMismatch { detail } => write!(f, "value-mismatch: {detail}"),
            Verdict::BothPanicked { detail } => write!(f, "both-panicked: {detail}"),
        }
    }
}

/// Compare the two engines' outcomes for one fixture.
///

/// `vbc` is engine A (VBC executor), `tree` is engine B (tree-walk
/// evaluator) — argument order is fixed and every verdict labels sides
/// explicitly.
pub fn compare_outcomes(vbc: &EngineOutcome, tree: &EngineOutcome) -> Verdict {
    match (vbc, tree) {
        (EngineOutcome::Panic(a), EngineOutcome::Panic(b)) => Verdict::BothPanicked {
            detail: format!("vbc panic: {a}; tree panic: {b}"),
        },
        (EngineOutcome::Value(a), EngineOutcome::Value(b)) => compare_values(a, b),
        (EngineOutcome::Error(_), EngineOutcome::Error(_)) => {
            // Shape-level agreement; see module docs for why messages are
            // not compared across disjoint error taxonomies.
            Verdict::Agree
        }
        (a, b) => Verdict::OutcomeMismatch {
            vbc: a.kind(),
            tree: b.kind(),
            detail: format!("vbc: {a}; tree: {b}"),
        },
    }
}

fn compare_values(vbc: &Comparable, tree: &Comparable) -> Verdict {
    // Opaque never participates in value comparison — including opaque
    // LEAVES inside structurally-decoded containers (a list of AST nodes
    // decodes as a Seq of Opaques; its container shape alone proves
    // nothing about value agreement).
    match (vbc, tree) {
        (Comparable::Opaque(a), Comparable::Opaque(b)) => Verdict::OpaqueBoth {
            vbc: (*a).to_string(),
            tree: (*b).to_string(),
        },
        _ if vbc.is_opaque() || tree.is_opaque() => Verdict::TypeMismatch {
            vbc_kind: vbc.kind(),
            tree_kind: tree.kind(),
            detail: format!("vbc: {vbc}; tree: {tree}"),
        },
        _ if vbc.contains_opaque() || tree.contains_opaque() => Verdict::OpaqueBoth {
            vbc: format!("opaque-leaf in {vbc}"),
            tree: format!("opaque-leaf in {tree}"),
        },
        _ if vbc.kind() != tree.kind() => Verdict::TypeMismatch {
            vbc_kind: vbc.kind(),
            tree_kind: tree.kind(),
            detail: format!("vbc: {vbc}; tree: {tree}"),
        },
        _ => {
            if structural_eq(vbc, tree) {
                Verdict::Agree
            } else {
                Verdict::ValueMismatch {
                    detail: format!("vbc: {vbc}; tree: {tree}"),
                }
            }
        }
    }
}

/// Recursive value equality over the comparable domain: floats compare
/// with [`float_eq`] at every depth (a `List<Float>` deserves the same
/// epsilon treatment as a bare `Float`); sequences require matching kind
/// labels, length, and element-wise equality; maps compare their
/// canonically-sorted entry lists pairwise. Scalars fall back to
/// structural `PartialEq`.
fn structural_eq(a: &Comparable, b: &Comparable) -> bool {
    match (a, b) {
        (Comparable::Float(x), Comparable::Float(y)) => float_eq(*x, *y),
        (Comparable::Seq(ka, xs), Comparable::Seq(kb, ys)) => {
            ka == kb
                && xs.len() == ys.len()
                && xs.iter().zip(ys.iter()).all(|(x, y)| structural_eq(x, y))
        }
        (Comparable::MapV(xs), Comparable::MapV(ys)) => {
            xs.len() == ys.len()
                && xs.iter().zip(ys.iter()).all(|((kx, vx), (ky, vy))| {
                    structural_eq(kx, ky) && structural_eq(vx, vy)
                })
        }
        _ => a == b,
    }
}
