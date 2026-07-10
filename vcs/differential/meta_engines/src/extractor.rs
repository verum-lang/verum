//! Extractors: engine-native results → the comparable value domain.
//!

//! Two independent value models feed the harness:
//!

//! * **VBC**: a NaN-boxed [`verum_vbc::value::Value`] whose heap-backed
//!   payloads (Text, List, Map, records) live in the interpreter heap and
//!   are only decodable through
//!   [`verum_vbc::interpreter::InterpreterState`].
//! * **Tree-walk**: a [`verum_ast::MetaValue`] (the compiler's `ConstValue`
//!   alias) — a plain Rust enum, `i128`-precision integers.
//!

//! Both are folded into [`Comparable`]: `Unit` / `Bool` / `Int(i128)` /
//! `Float` / `Char` / `Text` decoded **exactly**; collections decode
//! **structurally** (step-(ii) of ARCH-P4, donor
//! `verum_vbc::interpreter::script_engine::extract_owned` for the scalar
//! discrimination order, `InterpreterState::{list_elements, map_entries,
//! record_named_fields}` for the heap walks):
//!

//! * Lists / arrays → [`Comparable::Seq`] with kind `"seq-list"`.
//! * Records → [`Comparable::Seq`] with kind `"seq-tuple"`, field values
//!   in DECLARED order on the VBC side — deliberately matching the
//!   tree-walk evaluator's record arm, which degrades records to tuples
//!   of field values (evaluator.rs `MetaExpr::Record`). The tree-walk
//!   builds that tuple in SOURCE (literal) order, so a literal written
//!   out of declaration order is a REAL engine divergence — pinned by
//!   fixture, not papered over here.
//! * Maps → [`Comparable::MapV`], entries sorted by key display form —
//!   VBC map slots sit in hash order, the tree-walk's `OrderedMap` in
//!   insertion order; canonical sorting compares CONTENT, not layout.
//!
//! Everything else — AST nodes, sets, sum-type variants, out-of-range
//! `UInt`, unknown heap shapes — stays [`Comparable::Opaque`] and is
//! **never value-compared**.
//!

//! ## Deliberate normalizations (documented, not hidden)
//!

//! * VBC `i64` integers widen losslessly into the `Int(i128)` domain.
//! * Tree-walk `UInt(u128)` folds into `Int(i128)` when it fits — the
//!   engines disagree about signedness internally (`UInt` exists only in the
//!   tree-walk model; e.g. its Text `len` returns `UInt`), but the
//!   language-level value is the same integer. Out-of-`i128`-range `UInt`s
//!   become `Opaque` rather than mis-compare.
//! * VBC has **no Char representation** — char-typed results surface as
//!   codepoint integers. The extractor does not guess: they stay `Int`, and
//!   the char-vs-codepoint divergence is pinned by a fixture.
//! * `NaN` floats are decoded as floats on both sides (`is_nan_float` on the
//!   VBC side); NaN-vs-NaN equality is the comparator's decision, not ours.

use verum_ast::MetaValue;
use verum_vbc::interpreter::InterpreterState;
use verum_vbc::value::Value;

/// Recursion guard for structural decoding: meta values are finite by
/// construction, but a corrupt heap shape must not stack-overflow the
/// harness — past this depth the value decodes as `Opaque`.
const MAX_DEPTH: usize = 64;

/// The comparable value domain shared by both engines.
#[derive(Debug, Clone, PartialEq)]
pub enum Comparable {
    /// Unit / no meaningful value.
    Unit,
    /// Boolean.
    Bool(bool),
    /// Integer, widened to `i128` (the tree-walk's native precision; VBC's
    /// `i64` widens losslessly; tree-walk `UInt` folds in when it fits).
    Int(i128),
    /// IEEE-754 double.
    Float(f64),
    /// Unicode scalar — produced only by the tree-walk engine (VBC has no
    /// char tag; see module docs).
    Char(char),
    /// UTF-8 text, copied out of the owning engine.
    Text(String),
    /// Ordered sequence with an honest kind label: `"seq-list"` for
    /// lists/arrays, `"seq-tuple"` for tuples AND records (both engines
    /// reduce records to field-value tuples; see module docs). The label
    /// participates in kind comparison, so a list never equals a tuple.
    Seq(&'static str, Vec<Comparable>),
    /// Map entries sorted by key display form (canonical, layout-free).
    MapV(Vec<(Comparable, Comparable)>),
    /// Anything the harness cannot faithfully decode (AST nodes, sets,
    /// sum-type variants, out-of-range `UInt`, unknown heap shapes). The
    /// label says *what kind* of opaque value was seen; opaque values are
    /// never value-compared.
    Opaque(&'static str),
}

impl Comparable {
    /// Stable kind label used in verdicts and pin shapes.
    pub fn kind(&self) -> &'static str {
        match self {
            Comparable::Unit => "Unit",
            Comparable::Bool(_) => "Bool",
            Comparable::Int(_) => "Int",
            Comparable::Float(_) => "Float",
            Comparable::Char(_) => "Char",
            Comparable::Text(_) => "Text",
            Comparable::Seq(kind, _) => kind,
            Comparable::MapV(_) => "Map",
            Comparable::Opaque(_) => "Opaque",
        }
    }

    /// True for [`Comparable::Opaque`].
    pub fn is_opaque(&self) -> bool {
        matches!(self, Comparable::Opaque(_))
    }

    /// True if any node in this value is [`Comparable::Opaque`] — a
    /// structurally-decoded container with an opaque leaf must not
    /// value-compare (the leaf carries unknown semantics).
    pub fn contains_opaque(&self) -> bool {
        match self {
            Comparable::Opaque(_) => true,
            Comparable::Seq(_, items) => items.iter().any(|i| i.contains_opaque()),
            Comparable::MapV(entries) => entries
                .iter()
                .any(|(k, v)| k.contains_opaque() || v.contains_opaque()),
            _ => false,
        }
    }
}

impl std::fmt::Display for Comparable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Comparable::Unit => write!(f, "Unit"),
            Comparable::Bool(b) => write!(f, "Bool({b})"),
            Comparable::Int(i) => write!(f, "Int({i})"),
            Comparable::Float(x) => write!(f, "Float({x:?})"),
            Comparable::Char(c) => write!(f, "Char({c:?})"),
            Comparable::Text(t) => write!(f, "Text({t:?})"),
            Comparable::Seq(kind, items) => {
                write!(f, "{kind}[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, "]")
            }
            Comparable::MapV(entries) => {
                write!(f, "map{{")?;
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{k}: {v}")?;
                }
                write!(f, "}}")
            }
            Comparable::Opaque(k) => write!(f, "Opaque[{k}]"),
        }
    }
}

/// Sort map entries by the key's display form — the shared canonical
/// order for both engines (see module docs).
fn sort_map_entries(entries: &mut [(Comparable, Comparable)]) {
    entries.sort_by(|(a, _), (b, _)| a.to_string().cmp(&b.to_string()));
}

/// Decode a VBC result [`Value`] against the interpreter state that produced
/// it.
///

/// Discrimination order mirrors the step-(ii) donor
/// (`script_engine::extract_owned`) exactly: unit/nil → bool → int →
/// float (incl. NaN) → heap text → list → map → record → unknown.
pub fn from_vbc(state: &InterpreterState, value: Value) -> Comparable {
    from_vbc_depth(state, value, 0)
}

fn from_vbc_depth(state: &InterpreterState, value: Value, depth: usize) -> Comparable {
    if depth > MAX_DEPTH {
        return Comparable::Opaque("vbc-depth-limit");
    }
    if value.is_unit() || value.is_nil() {
        Comparable::Unit
    } else if value.is_bool() {
        Comparable::Bool(value.as_bool())
    } else if value.is_int() {
        Comparable::Int(i128::from(value.as_i64()))
    } else if value.is_float() || value.is_nan_float() {
        // `is_nan_float` catches `from_f64(NaN)` results (0.0/0.0, …) that
        // `is_float()` excludes but `as_f64()` decodes — same guard as the
        // donor, otherwise NaN results mis-classify as unknown.
        Comparable::Float(value.as_f64())
    } else if let Some(s) = state.read_text(value) {
        Comparable::Text(s)
    } else if let Some(elems) = state.list_elements(value) {
        Comparable::Seq(
            "seq-list",
            elems
                .into_iter()
                .map(|e| from_vbc_depth(state, e, depth + 1))
                .collect(),
        )
    } else if let Some(entries) = state.map_entries(value) {
        let mut out: Vec<(Comparable, Comparable)> = entries
            .into_iter()
            .map(|(k, v)| {
                (
                    from_vbc_depth(state, k, depth + 1),
                    from_vbc_depth(state, v, depth + 1),
                )
            })
            .collect();
        sort_map_entries(&mut out);
        Comparable::MapV(out)
    } else if let Some(fields) = state.record_named_fields(value) {
        // Records → field-value tuple in DECLARED order (module docs).
        Comparable::Seq(
            "seq-tuple",
            fields
                .into_iter()
                .map(|(_name, v)| from_vbc_depth(state, v, depth + 1))
                .collect(),
        )
    } else {
        Comparable::Opaque("vbc-unknown")
    }
}

/// Decode a tree-walk result (`ConstValue` = [`MetaValue`]).
pub fn from_tree_walk(value: &MetaValue) -> Comparable {
    from_tree_walk_depth(value, 0)
}

fn from_tree_walk_depth(value: &MetaValue, depth: usize) -> Comparable {
    if depth > MAX_DEPTH {
        return Comparable::Opaque("tree-depth-limit");
    }
    match value {
        MetaValue::Unit => Comparable::Unit,
        MetaValue::Bool(b) => Comparable::Bool(*b),
        MetaValue::Int(i) => Comparable::Int(*i),
        MetaValue::UInt(u) => {
            // Signedness is an engine-internal distinction (see module
            // docs); fold into the Int domain when lossless.
            if *u <= i128::MAX as u128 {
                Comparable::Int(*u as i128)
            } else {
                Comparable::Opaque("tree-uint-out-of-i128-range")
            }
        }
        MetaValue::Float(f) => Comparable::Float(*f),
        MetaValue::Char(c) => Comparable::Char(*c),
        MetaValue::Text(t) => Comparable::Text(t.as_str().to_string()),
        MetaValue::Bytes(_) => Comparable::Opaque("tree-bytes"),
        MetaValue::Array(items) => Comparable::Seq(
            "seq-list",
            items
                .iter()
                .map(|i| from_tree_walk_depth(i, depth + 1))
                .collect(),
        ),
        MetaValue::Tuple(items) => Comparable::Seq(
            "seq-tuple",
            items
                .iter()
                .map(|i| from_tree_walk_depth(i, depth + 1))
                .collect(),
        ),
        MetaValue::Maybe(_) => Comparable::Opaque("tree-maybe"),
        MetaValue::Map(map) => {
            let mut out: Vec<(Comparable, Comparable)> = map
                .iter()
                .map(|(k, v)| {
                    (
                        Comparable::Text(k.as_str().to_string()),
                        from_tree_walk_depth(v, depth + 1),
                    )
                })
                .collect();
            sort_map_entries(&mut out);
            Comparable::MapV(out)
        }
        MetaValue::Set(_) => Comparable::Opaque("tree-set"),
        MetaValue::Expr(_) => Comparable::Opaque("tree-ast-expr"),
        MetaValue::Type(_) => Comparable::Opaque("tree-ast-type"),
        MetaValue::Pattern(_) => Comparable::Opaque("tree-ast-pattern"),
        MetaValue::Item(_) => Comparable::Opaque("tree-ast-item"),
        MetaValue::Items(_) => Comparable::Opaque("tree-ast-items"),
    }
}
