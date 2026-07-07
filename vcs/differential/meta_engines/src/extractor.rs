//! Extractors: engine-native results → the comparable scalar domain.
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
//! `Float` / `Char` / `Text` decoded **exactly**; everything else —
//! collections, tuples, AST nodes, records — becomes [`Comparable::Opaque`]
//! and is **never value-compared**. Faithful structural decoding of VBC
//! collections is the step-(ii) follow-up; its donor implementation is
//! `verum_vbc::interpreter::script_engine::extract_owned`, which already
//! marshals List/Map structurally for the `core.script` embedding surface
//! (this extractor mirrors its scalar discrimination order exactly).
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

/// The comparable scalar domain shared by both engines.
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
    /// Anything the harness cannot faithfully decode (collections, tuples,
    /// records, AST nodes, out-of-range `UInt`, unknown heap shapes). The
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
            Comparable::Opaque(_) => "Opaque",
        }
    }

    /// True for [`Comparable::Opaque`].
    pub fn is_opaque(&self) -> bool {
        matches!(self, Comparable::Opaque(_))
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
            Comparable::Opaque(k) => write!(f, "Opaque[{k}]"),
        }
    }
}

/// Decode a VBC result [`Value`] against the interpreter state that produced
/// it.
///

/// Discrimination order mirrors the step-(ii) donor
/// (`script_engine::extract_owned`) exactly: unit/nil → bool → int →
/// float (incl. NaN) → heap text → list → map → unknown. Lists and maps are
/// *detected* (so the label is honest) but not decoded — they are `Opaque`
/// by the extractor contract.
pub fn from_vbc(state: &InterpreterState, value: Value) -> Comparable {
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
    } else if state.list_elements(value).is_some() {
        Comparable::Opaque("vbc-list")
    } else if state.map_entries(value).is_some() {
        Comparable::Opaque("vbc-map")
    } else {
        Comparable::Opaque("vbc-unknown")
    }
}

/// Decode a tree-walk result (`ConstValue` = [`MetaValue`]).
pub fn from_tree_walk(value: &MetaValue) -> Comparable {
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
        MetaValue::Array(_) => Comparable::Opaque("tree-array"),
        MetaValue::Tuple(_) => Comparable::Opaque("tree-tuple"),
        MetaValue::Maybe(_) => Comparable::Opaque("tree-maybe"),
        MetaValue::Map(_) => Comparable::Opaque("tree-map"),
        MetaValue::Set(_) => Comparable::Opaque("tree-set"),
        MetaValue::Expr(_) => Comparable::Opaque("tree-ast-expr"),
        MetaValue::Type(_) => Comparable::Opaque("tree-ast-type"),
        MetaValue::Pattern(_) => Comparable::Opaque("tree-ast-pattern"),
        MetaValue::Item(_) => Comparable::Opaque("tree-ast-item"),
        MetaValue::Items(_) => Comparable::Opaque("tree-ast-items"),
    }
}
