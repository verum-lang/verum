//! Property-based testing runner.
//!

//! A `@property` function differs from `@test` in that the runner calls
//! it N times with randomly-generated inputs rather than once with no
//! arguments. On failure the harness performs Hedgehog-style integrated
//! shrinking to produce a minimal counter-example, records the failing
//! seed in `target/test/pbt-regressions.json`, and replays stored seeds
//! first on subsequent runs (Hypothesis convention).
//!

//! Design rationale lives in `docs/testing/reference-quality-roadmap.md`;
//! the TL;DR is:
//!

//!  * Runner is Rust-side (owns the VBC interpreter, AST, RNG).
//!  * Each generator produces a lazy rose-tree `Tree<T>` — integrated
//!  shrinking means `.value` and `.shrinks` can never disagree, and
//!  refinement-type bounds are respected by construction.
//!  * The runner emits a one-line replay command on failure so CI
//!  output copy-pasted into a terminal reproduces the bug.
//!  * Regression DB is best-effort: corrupt or missing file is a
//!  warning, not a hard error — we always want fresh seeds to work.

use crate::error::{CliError, Result};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use verum_ast::{FunctionParamKind, GenericArg, ItemKind, PathSegment, TypeKind};
use verum_vbc::{FunctionId, Value, VbcModule};

// --------------------------------------------------------------------
// RNG — tiny self-contained linear-congruential + splitmix mixer
// --------------------------------------------------------------------

/// Seed = 64-bit state. We use SplitMix64 (Steele et al., 2014) — fast,
/// decent statistical quality, deterministic, one state word. Good
/// enough for test-input generation; *not* cryptographic.
#[derive(Debug, Clone, Copy)]
pub struct Seed(pub u64);

impl Seed {
    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.trim_start_matches("0x").trim_start_matches("0X");
        u64::from_str_radix(s, 16).ok().map(Seed)
    }
    pub fn to_hex(self) -> String {
        format!("0x{:016x}", self.0)
    }
}

/// Split a seed into two independent streams so we can feed distinct
/// generators without correlation (one per function parameter).
fn split(seed: Seed) -> (Seed, Seed) {
    // Derive child seeds with two different mixes of the parent.
    let mix = |x: u64| {
        let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    (Seed(mix(seed.0)), Seed(mix(seed.0.wrapping_add(1))))
}

/// Draw the next u64 from `seed`, producing the updated state.
fn next_u64(seed: &mut Seed) -> u64 {
    seed.0 = seed.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = seed.0;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn rand_range_i64(seed: &mut Seed, lo: i64, hi: i64) -> i64 {
    if hi <= lo {
        return lo;
    }
    let span = (hi - lo + 1) as u64;
    let r = next_u64(seed) % span;
    lo.wrapping_add(r as i64)
}

fn rand_range_u32(seed: &mut Seed, lo: u32, hi: u32) -> u32 {
    if hi <= lo {
        return lo;
    }
    let span = (hi - lo + 1) as u64;
    let r = next_u64(seed) % span;
    lo + r as u32
}

// --------------------------------------------------------------------
// Rose tree for integrated shrinking (Hedgehog-style)
// --------------------------------------------------------------------

/// Rose tree: a generated value together with the set of "smaller"
/// values we'd try during shrinking. Shrinks are represented as
/// *recipes* (closures) rather than eager data to keep construction
/// O(1) and support arbitrarily deep shrink trees without blowing up
/// memory.
pub struct Tree<T: Clone> {
    pub value: T,
    pub shrinks: Vec<Box<dyn Fn() -> Tree<T> + Send + Sync>>,
}

impl<T: Clone> Tree<T> {
    pub fn singleton(value: T) -> Self {
        Tree {
            value,
            shrinks: Vec::new(),
        }
    }
    pub fn new(value: T, shrinks: Vec<Box<dyn Fn() -> Tree<T> + Send + Sync>>) -> Self {
        Tree { value, shrinks }
    }
}

// --------------------------------------------------------------------
// Generators for primitives & simple types
// --------------------------------------------------------------------

/// A runtime-dispatched generator — one variant per supported Verum type.
/// Keeps the public surface small; internal shrink logic lives in the
/// producer helpers below.
///
/// Compound-type variants (Maybe, Result, List, Tuple) are recursive:
/// each carries `Box<Generator>` for the element/field generators so
/// arbitrary nesting is supported. `Fallback` is used when the type
/// cannot be structurally decomposed; it produces `Value::unit()` and
/// emits a tracing warning so the test still runs but the user knows.
pub enum Generator {
    Bool,
    /// Full Int range.
    Int,
    /// Bounded Int from refinement type `Int{ lo <= it <= hi }`.
    IntRange { lo: i64, hi: i64 },
    /// Non-negative integers.
    Nat,
    /// IEEE 754 f64 with edge-cases bias.
    Float,
    /// Text with length bound.
    Text { max_len: u32 },
    /// `Maybe<T>` — 50 % None, 50 % Some(inner).
    Maybe(Box<Generator>),
    /// `Result<T, E>` — 50 % Ok(ok_gen), 50 % Err(err_gen).
    Result { ok: Box<Generator>, err: Box<Generator> },
    /// `List<T>` — random length in `[0, max_len]`, each element from `elem`.
    List { elem: Box<Generator>, max_len: u32 },
    /// `Ordering` — uniform over Less / Equal / Greater (tags 0 / 1 / 2).
    Ordering,
    /// Tuple `(T0, T1, …)` — product of per-field generators.
    Tuple(Vec<Generator>),
    /// Unknown / unsupported type — produces `Value::unit()` with a warning.
    Fallback,
}

// --------------------------------------------------------------------
// Helper: extract the leaf name from a path type
// --------------------------------------------------------------------

fn path_leaf<'a>(ty: &'a verum_ast::Type) -> Option<&'a str> {
    match &ty.kind {
        TypeKind::Path(p) => p.segments.last().and_then(|s| match s {
            PathSegment::Name(id) => Some(id.name.as_str()),
            _ => None,
        }),
        _ => None,
    }
}

fn generic_arg_ty(arg: &GenericArg) -> Option<&verum_ast::Type> {
    match arg {
        GenericArg::Type(t) => Some(t),
        _ => None,
    }
}

impl Generator {
    /// Build a generator for a Verum AST type.
    ///
    /// Never returns `None` — unknown types produce `Generator::Fallback`
    /// (which emits a `tracing::warn!` when sampled and yields `Value::unit()`).
    /// This guarantees every `@property` test runs regardless of parameter types.
    pub fn for_type(ty: &verum_ast::Type) -> Option<Self> {
        Some(Self::for_type_inner(ty))
    }

    fn for_type_inner(ty: &verum_ast::Type) -> Self {
        // Strip outer references (&T, &mut T, &checked T) transparently.
        match &ty.kind {
            TypeKind::Reference { inner, .. }
            | TypeKind::CheckedReference { inner, .. }
            | TypeKind::UnsafeReference { inner, .. } => {
                return Self::for_type_inner(inner);
            }
            _ => {}
        }

        // Unwrap refinement wrapper to reach the base kind.
        let base_kind = unwrap_refinement(&ty.kind);
        let bounds = extract_bounds(&ty.kind);

        match base_kind {
            // ---- primitives ----
            TypeKind::Bool => Generator::Bool,
            TypeKind::Int => match bounds {
                Some((lo, hi)) => Generator::IntRange { lo, hi },
                None => Generator::Int,
            },
            TypeKind::Float => Generator::Float,
            TypeKind::Text => Generator::Text { max_len: 32 },
            TypeKind::Unit => Generator::Tuple(vec![]),  // unit == empty tuple

            // ---- named / path types ----
            TypeKind::Path(p) => {
                let name = p.segments.last().and_then(|s| match s {
                    PathSegment::Name(id) => Some(id.name.as_str()),
                    _ => None,
                });
                match name {
                    Some("Nat") | Some("U8") | Some("U16") | Some("U32") | Some("U64") => {
                        Generator::Nat
                    }
                    Some("I8") | Some("I16") | Some("I32") | Some("I64") | Some("Int") => {
                        Generator::Int
                    }
                    Some("Bool") => Generator::Bool,
                    Some("Byte") => Generator::IntRange { lo: 0, hi: 255 },
                    Some("Float") | Some("F32") | Some("F64") => Generator::Float,
                    Some("Text") | Some("String") => Generator::Text { max_len: 32 },
                    Some("Ordering") => Generator::Ordering,
                    _ => Generator::Fallback,
                }
            }

            // ---- generic types: Maybe<T>, Result<T,E>, List<T>, … ----
            TypeKind::Generic { base, args } => {
                let name = path_leaf(base);
                match name {
                    Some("Maybe") => {
                        let inner = args
                            .first()
                            .and_then(generic_arg_ty)
                            .map(Self::for_type_inner)
                            .unwrap_or(Generator::Fallback);
                        Generator::Maybe(Box::new(inner))
                    }
                    Some("Result") => {
                        let ok = args
                            .first()
                            .and_then(generic_arg_ty)
                            .map(Self::for_type_inner)
                            .unwrap_or(Generator::Fallback);
                        let err = args
                            .get(1)
                            .and_then(generic_arg_ty)
                            .map(Self::for_type_inner)
                            .unwrap_or(Generator::Fallback);
                        Generator::Result {
                            ok: Box::new(ok),
                            err: Box::new(err),
                        }
                    }
                    Some("List") => {
                        let elem = args
                            .first()
                            .and_then(generic_arg_ty)
                            .map(Self::for_type_inner)
                            .unwrap_or(Generator::Fallback);
                        Generator::List {
                            elem: Box::new(elem),
                            max_len: 8,
                        }
                    }
                    // Transparent ownership wrappers — generate the inner T.
                    Some("Heap") | Some("Shared") => args
                        .first()
                        .and_then(generic_arg_ty)
                        .map(Self::for_type_inner)
                        .unwrap_or(Generator::Fallback),
                    _ => Generator::Fallback,
                }
            }

            // ---- tuple types ----
            TypeKind::Tuple(types) => {
                let field_gens: Vec<Generator> =
                    types.iter().map(Self::for_type_inner).collect();
                Generator::Tuple(field_gens)
            }

            _ => Generator::Fallback,
        }
    }

    /// Sample a rose tree on a dedicated seed.
    pub fn sample(&self, seed: &mut Seed) -> TreeValue {
        match self {
            Generator::Bool => gen_bool(seed),
            Generator::Int => gen_int(seed),
            Generator::IntRange { lo, hi } => gen_int_range(seed, *lo, *hi),
            Generator::Nat => gen_int_range(seed, 0, 1_000_000),
            Generator::Float => gen_float(seed),
            Generator::Text { max_len } => gen_text(seed, *max_len),
            Generator::Maybe(inner) => gen_maybe(seed, inner),
            Generator::Result { ok, err } => gen_result(seed, ok, err),
            Generator::List { elem, max_len } => gen_list(seed, elem, *max_len),
            Generator::Ordering => gen_ordering(seed),
            Generator::Tuple(gens) => gen_tuple(seed, gens),
            Generator::Fallback => {
                tracing::warn!(
                    "property generator: unknown type — using unit fallback. \
                     Add a Generator variant for this type to exercise it properly."
                );
                TreeValue::Tuple(vec![])
            }
        }
    }
}

/// Dynamically-typed generated value, carrying the bounds that its
/// generator was constrained by so shrinks can't escape the refinement
/// domain. A fresh `Int` generator uses `[i64::MIN, i64::MAX]`; a
/// refined `Int{ 1 <= it <= 100 }` uses `[1, 100]`.
///
/// Compound variants mirror the language types they represent:
/// `Maybe(None)` = `Maybe::None`, `Maybe(Some(v))` = `Maybe::Some(v)`,
/// `Ordering(-1/0/1)` = `Less/Equal/Greater`, `List(elems)` = a Verum
/// List, `Tuple(fields)` = an anonymous tuple.
#[derive(Debug, Clone, PartialEq)]
pub enum TreeValue {
    // ---- primitives ----
    Bool(bool),
    Int { value: i64, lo: i64, hi: i64 },
    Float(f64),
    Text { value: String, max_len: u32 },
    // ---- compound types ----
    /// `Maybe<T>` — `None` represented as `Maybe(None)`.
    Maybe(Option<Box<TreeValue>>),
    /// `Result<T,E>` — `is_ok=true` → Ok(inner), `is_ok=false` → Err(inner).
    ResultVal { is_ok: bool, inner: Box<TreeValue> },
    /// `List<T>` — variable-length vector of element values.
    List(Vec<TreeValue>),
    /// `Ordering` — -1 = Less (tag 0), 0 = Equal (tag 1), 1 = Greater (tag 2).
    Ordering(i8),
    /// Tuple / unit — empty vec for `()`, otherwise field values in order.
    Tuple(Vec<TreeValue>),
}

impl TreeValue {
    pub fn display(&self) -> String {
        match self {
            TreeValue::Bool(b) => b.to_string(),
            TreeValue::Int { value, .. } => value.to_string(),
            TreeValue::Float(f) => format!("{:?}", f),
            TreeValue::Text { value, .. } => format!("{:?}", value),
            TreeValue::Maybe(None) => "None".to_string(),
            TreeValue::Maybe(Some(v)) => format!("Some({})", v.display()),
            TreeValue::ResultVal { is_ok: true, inner } => format!("Ok({})", inner.display()),
            TreeValue::ResultVal { is_ok: false, inner } => format!("Err({})", inner.display()),
            TreeValue::List(elems) => format!(
                "[{}]",
                elems.iter().map(TreeValue::display).collect::<Vec<_>>().join(", ")
            ),
            TreeValue::Ordering(-1) => "Less".to_string(),
            TreeValue::Ordering(0) => "Equal".to_string(),
            TreeValue::Ordering(_) => "Greater".to_string(),
            TreeValue::Tuple(fields) if fields.is_empty() => "()".to_string(),
            TreeValue::Tuple(fields) => format!(
                "({})",
                fields.iter().map(TreeValue::display).collect::<Vec<_>>().join(", ")
            ),
        }
    }

    /// One step of shrinking — returns candidates *closer to a minimal
    /// case* than self. Empty vec means "already minimal".
    ///
    /// Shrinks are filtered to stay within the generator's domain:
    /// a refined `Int{ 1..=100 }` never shrinks to 0, preserving the
    /// refinement invariant by construction.
    ///
    /// Compound shrinking follows the Hedgehog convention: always try
    /// to eliminate structure first (None < Some, Err < Ok, [] < [x]).
    pub fn shrink(&self) -> Vec<TreeValue> {
        match self {
            // ---- primitives ----
            TreeValue::Bool(true) => vec![TreeValue::Bool(false)],
            TreeValue::Bool(false) => vec![],
            TreeValue::Int { value: 0, .. } => vec![],
            TreeValue::Int { value, lo, hi } => shrink_int(*value)
                .into_iter()
                .filter(|v| v >= lo && v <= hi)
                .map(|v| TreeValue::Int { value: v, lo: *lo, hi: *hi })
                .collect(),
            TreeValue::Float(f) if *f == 0.0 => vec![],
            TreeValue::Float(f) => shrink_float(*f).into_iter().map(TreeValue::Float).collect(),
            TreeValue::Text { value, .. } if value.is_empty() => vec![],
            TreeValue::Text { value, max_len } => shrink_text(value)
                .into_iter()
                .filter(|s| s.chars().count() as u32 <= *max_len)
                .map(|s| TreeValue::Text { value: s, max_len: *max_len })
                .collect(),
            // ---- compound ----
            TreeValue::Maybe(None) => vec![],
            TreeValue::Maybe(Some(inner)) => {
                // Minimal: None; then shrink the payload.
                let mut out = vec![TreeValue::Maybe(None)];
                for s in inner.shrink() {
                    out.push(TreeValue::Maybe(Some(Box::new(s))));
                }
                out
            }
            TreeValue::ResultVal { is_ok: false, inner } => {
                // Err shrinks toward Ok (simpler); then shrink payload.
                let mut out = vec![TreeValue::ResultVal { is_ok: true, inner: inner.clone() }];
                for s in inner.shrink() {
                    out.push(TreeValue::ResultVal { is_ok: false, inner: Box::new(s) });
                }
                out
            }
            TreeValue::ResultVal { is_ok: true, inner } => {
                inner.shrink()
                    .into_iter()
                    .map(|s| TreeValue::ResultVal { is_ok: true, inner: Box::new(s) })
                    .collect()
            }
            TreeValue::List(elems) if elems.is_empty() => vec![],
            TreeValue::List(elems) => shrink_list(elems),
            TreeValue::Ordering(-1) | TreeValue::Ordering(1) => {
                vec![TreeValue::Ordering(0)]  // Less/Greater → Equal (minimal)
            }
            TreeValue::Ordering(_) => vec![],  // Equal is minimal
            TreeValue::Tuple(fields) if fields.is_empty() => vec![],
            TreeValue::Tuple(fields) => shrink_tuple(fields),
        }
    }

    pub fn to_vbc_value(&self, interp: &mut verum_vbc::interpreter::Interpreter) -> Result<Value> {
        use verum_common::well_known_types::{
            maybe_none_tag, maybe_success_tag, ordering_tag_for_std,
            result_error_tag, result_success_tag,
        };
        Ok(match self {
            TreeValue::Bool(b) => Value::from_bool(*b),
            TreeValue::Int { value, .. } => Value::from_i64(*value),
            TreeValue::Float(f) => Value::from_f64(*f),
            TreeValue::Text { value, .. } => interp
                .alloc_string(value)
                .map_err(|e| CliError::RuntimeError(format!("alloc_string: {:?}", e)))?,
            TreeValue::Maybe(None) => interp
                .alloc_variant(maybe_none_tag(), &[])
                .map_err(|e| CliError::RuntimeError(format!("alloc_variant(None): {:?}", e)))?,
            TreeValue::Maybe(Some(inner)) => {
                let v = inner.to_vbc_value(interp)?;
                interp
                    .alloc_variant(maybe_success_tag(), &[v])
                    .map_err(|e| CliError::RuntimeError(format!("alloc_variant(Some): {:?}", e)))?
            }
            TreeValue::ResultVal { is_ok: true, inner } => {
                let v = inner.to_vbc_value(interp)?;
                interp
                    .alloc_variant(result_success_tag(), &[v])
                    .map_err(|e| CliError::RuntimeError(format!("alloc_variant(Ok): {:?}", e)))?
            }
            TreeValue::ResultVal { is_ok: false, inner } => {
                let v = inner.to_vbc_value(interp)?;
                interp
                    .alloc_variant(result_error_tag(), &[v])
                    .map_err(|e| CliError::RuntimeError(format!("alloc_variant(Err): {:?}", e)))?
            }
            TreeValue::List(elems) => {
                let vs: Vec<Value> = elems
                    .iter()
                    .map(|e| e.to_vbc_value(interp))
                    .collect::<Result<Vec<_>>>()?;
                interp
                    .alloc_list(&vs)
                    .map_err(|e| CliError::RuntimeError(format!("alloc_list: {:?}", e)))?
            }
            TreeValue::Ordering(ord) => {
                // -1 → Less, 0 → Equal, 1 (or anything else) → Greater
                // matches what `Ord::cmp` returns at the std level, then
                // translates through the canonical Verum layout.
                let std_ord = match ord {
                    -1 => std::cmp::Ordering::Less,
                    0 => std::cmp::Ordering::Equal,
                    _ => std::cmp::Ordering::Greater,
                };
                interp
                    .alloc_variant(ordering_tag_for_std(std_ord), &[])
                    .map_err(|e| CliError::RuntimeError(format!("alloc_variant(Ordering): {:?}", e)))?
            }
            TreeValue::Tuple(fields) => {
                let vs: Vec<Value> = fields
                    .iter()
                    .map(|f| f.to_vbc_value(interp))
                    .collect::<Result<Vec<_>>>()?;
                interp
                    .alloc_tuple(&vs)
                    .map_err(|e| CliError::RuntimeError(format!("alloc_tuple: {:?}", e)))?
            }
        })
    }
}

fn gen_bool(seed: &mut Seed) -> TreeValue {
    let v = (next_u64(seed) & 1) == 1;
    TreeValue::Bool(v)
}

fn gen_int(seed: &mut Seed) -> TreeValue {
    // 15% chance of picking an edge case (0, 1, -1, min, max) that
    // randoms rarely hit. Matches Hypothesis / hedgehog defaults.
    if rand_range_u32(seed, 0, 99) < 15 {
        let edges = [0i64, 1, -1, i64::MIN, i64::MAX, 2, -2, 100, -100];
        let i = rand_range_u32(seed, 0, (edges.len() - 1) as u32) as usize;
        return TreeValue::Int {
            value: edges[i],
            lo: i64::MIN,
            hi: i64::MAX,
        };
    }
    // Biased toward small magnitudes: pick a magnitude exponentially,
    // then a sign. Produces a mix of near-zero and large values.
    let bits = rand_range_u32(seed, 0, 63);
    let mag = if bits == 0 {
        0
    } else {
        next_u64(seed) & ((1u64 << bits) - 1)
    };
    let sign = (next_u64(seed) & 1) == 1;
    let v = if sign { -(mag as i64) } else { mag as i64 };
    TreeValue::Int {
        value: v,
        lo: i64::MIN,
        hi: i64::MAX,
    }
}

fn gen_int_range(seed: &mut Seed, lo: i64, hi: i64) -> TreeValue {
    let v = rand_range_i64(seed, lo, hi);
    TreeValue::Int { value: v, lo, hi }
}

fn gen_float(seed: &mut Seed) -> TreeValue {
    if rand_range_u32(seed, 0, 99) < 15 {
        let edges = [
            0.0_f64,
            -0.0,
            1.0,
            -1.0,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
            f64::MIN_POSITIVE,
            f64::EPSILON,
            f64::MAX,
            f64::MIN,
        ];
        let i = rand_range_u32(seed, 0, (edges.len() - 1) as u32) as usize;
        return TreeValue::Float(edges[i]);
    }
    let bits = next_u64(seed);
    TreeValue::Float(f64::from_bits(bits))
}

fn gen_text(seed: &mut Seed, max_len: u32) -> TreeValue {
    let len = rand_range_u32(seed, 0, max_len) as usize;
    let mut s = String::with_capacity(len);
    for _ in 0..len {
        // 80% ASCII printable, 20% "exotic" — pick from BMP incl. 4-byte.
        let pick = rand_range_u32(seed, 0, 99);
        let c = if pick < 80 {
            let cp = rand_range_u32(seed, 0x20, 0x7E);
            char::from_u32(cp).unwrap()
        } else {
            let options = [0x00, 0x7F, 0xA0, 0x1F600, 0x2603, 0x1F4A9, 0x4E2D, 0x0301];
            let idx = rand_range_u32(seed, 0, (options.len() - 1) as u32) as usize;
            char::from_u32(options[idx]).unwrap_or('?')
        };
        s.push(c);
    }
    TreeValue::Text { value: s, max_len }
}

// --------------------------------------------------------------------
// Compound-type generators
// --------------------------------------------------------------------

fn gen_maybe(seed: &mut Seed, inner: &Generator) -> TreeValue {
    if (next_u64(seed) & 1) == 0 {
        TreeValue::Maybe(None)
    } else {
        TreeValue::Maybe(Some(Box::new(inner.sample(seed))))
    }
}

fn gen_result(seed: &mut Seed, ok: &Generator, err: &Generator) -> TreeValue {
    if (next_u64(seed) & 1) == 0 {
        TreeValue::ResultVal { is_ok: true, inner: Box::new(ok.sample(seed)) }
    } else {
        TreeValue::ResultVal { is_ok: false, inner: Box::new(err.sample(seed)) }
    }
}

fn gen_list(seed: &mut Seed, elem: &Generator, max_len: u32) -> TreeValue {
    let len = rand_range_u32(seed, 0, max_len) as usize;
    let mut elems = Vec::with_capacity(len);
    for _ in 0..len {
        elems.push(elem.sample(seed));
    }
    TreeValue::List(elems)
}

fn gen_ordering(seed: &mut Seed) -> TreeValue {
    match rand_range_u32(seed, 0, 2) {
        0 => TreeValue::Ordering(-1),
        1 => TreeValue::Ordering(0),
        _ => TreeValue::Ordering(1),
    }
}

fn gen_tuple(seed: &mut Seed, gens: &[Generator]) -> TreeValue {
    let fields: Vec<TreeValue> = gens.iter().map(|g| g.sample(seed)).collect();
    TreeValue::Tuple(fields)
}

// --------------------------------------------------------------------
// Compound-type shrink helpers
// --------------------------------------------------------------------

fn shrink_list(elems: &[TreeValue]) -> Vec<TreeValue> {
    let mut out = Vec::new();
    // Empty list is minimal.
    out.push(TreeValue::List(vec![]));
    // Halve by dropping the second half.
    if elems.len() > 1 {
        out.push(TreeValue::List(elems[..elems.len() / 2].to_vec()));
    }
    // Drop one element at each position.
    for i in 0..elems.len() {
        let mut v = elems.to_vec();
        v.remove(i);
        out.push(TreeValue::List(v));
    }
    // Shrink the first element while keeping the rest.
    if let Some(first) = elems.first() {
        for s in first.shrink() {
            let mut v = elems.to_vec();
            v[0] = s;
            out.push(TreeValue::List(v));
        }
    }
    out.dedup_by(|a, b| a.display() == b.display());
    out
}

fn shrink_tuple(fields: &[TreeValue]) -> Vec<TreeValue> {
    let mut out = Vec::new();
    // Shrink each field independently.
    for (i, f) in fields.iter().enumerate() {
        for s in f.shrink() {
            let mut v = fields.to_vec();
            v[i] = s;
            out.push(TreeValue::Tuple(v));
        }
    }
    out
}

/// Shrink candidates for an integer — halve toward 0, also try nearby
/// small values. Classic QuickCheck strategy.
fn shrink_int(i: i64) -> Vec<i64> {
    if i == 0 {
        return vec![];
    }
    let mut out = vec![0];
    if i != i64::MIN {
        let n = i.wrapping_neg();
        if n.abs() < i.abs() {
            out.push(n);
        }
    }
    let mut cur = i / 2;
    while cur != 0 && cur != i {
        out.push(cur);
        cur /= 2;
    }
    if i > 0 {
        out.push(i - 1);
    } else if i < 0 {
        out.push(i + 1);
    }
    out.sort_by_key(|x| x.abs());
    out.dedup();
    out
}

fn shrink_float(f: f64) -> Vec<f64> {
    if f == 0.0 {
        return vec![];
    }
    let mut out = Vec::new();
    out.push(0.0);
    if f.is_finite() {
        let tr = f.trunc();
        if tr != f {
            out.push(tr);
        }
        let half = f / 2.0;
        if half != f && half.is_finite() {
            out.push(half);
        }
    } else {
        out.push(0.0);
    }
    out
}

fn shrink_text(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    if s.is_empty() {
        return out;
    }
    out.push(String::new());
    // Halve by length.
    let chars: Vec<char> = s.chars().collect();
    if chars.len() > 1 {
        let half: String = chars[..chars.len() / 2].iter().collect();
        out.push(half);
    }
    // Drop one char at each position.
    for i in 0..chars.len() {
        let mut c = chars.clone();
        c.remove(i);
        out.push(c.into_iter().collect());
    }
    out.dedup();
    out
}

// --------------------------------------------------------------------
// Refinement-type bound extraction (best-effort)
// --------------------------------------------------------------------

fn unwrap_refinement(kind: &TypeKind) -> &TypeKind {
    match kind {
        TypeKind::Refined { base, .. } => unwrap_refinement(&base.kind),
        other => other,
    }
}

/// Extract `(lo, hi)` from patterns like `Int{ 0 < it < 100 }`,
/// `Int{ it > 0 }`, `Int{ it <= 255 }` when they're simple integer
/// comparisons on `it`. Anything more complex returns `None` and we
/// fall back to the unbounded generator.
fn extract_bounds(kind: &TypeKind) -> Option<(i64, i64)> {
    let pred = match kind {
        TypeKind::Refined { predicate, .. } => &predicate.expr,
        _ => return None,
    };
    use verum_ast::{BinOp, Expr, ExprKind};
    let mut lo: i64 = i64::MIN;
    let mut hi: i64 = i64::MAX;
    fn walk(e: &Expr, lo: &mut i64, hi: &mut i64) -> bool {
        match &e.kind {
            ExprKind::Binary {
                op: BinOp::And,
                left,
                right,
            } => walk(left, lo, hi) && walk(right, lo, hi),
            ExprKind::Binary { op, left, right } => {
                let (it_left, value) = match (is_it_ref(left), lit_i64(right)) {
                    (true, Some(v)) => (true, v),
                    _ => match (lit_i64(left), is_it_ref(right)) {
                        (Some(v), true) => (false, v),
                        _ => return true, // ignore non-`it` predicates
                    },
                };
                match (op, it_left) {
                    (BinOp::Lt, true) => {
                        *hi = (*hi).min(value.saturating_sub(1));
                    }
                    (BinOp::Le, true) => {
                        *hi = (*hi).min(value);
                    }
                    (BinOp::Gt, true) => {
                        *lo = (*lo).max(value.saturating_add(1));
                    }
                    (BinOp::Ge, true) => {
                        *lo = (*lo).max(value);
                    }
                    (BinOp::Eq, _) => {
                        *lo = value;
                        *hi = value;
                    }
                    (BinOp::Lt, false) => {
                        *lo = (*lo).max(value.saturating_add(1));
                    }
                    (BinOp::Le, false) => {
                        *lo = (*lo).max(value);
                    }
                    (BinOp::Gt, false) => {
                        *hi = (*hi).min(value.saturating_sub(1));
                    }
                    (BinOp::Ge, false) => {
                        *hi = (*hi).min(value);
                    }
                    _ => {}
                }
                true
            }
            _ => true,
        }
    }
    fn is_it_ref(e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Path(p) => {
                if let [verum_ast::PathSegment::Name(id)] = p.segments.as_slice() {
                    id.name.as_str() == "it"
                } else {
                    false
                }
            }
            _ => false,
        }
    }
    fn lit_i64(e: &Expr) -> Option<i64> {
        match &e.kind {
            ExprKind::Literal(lit) => {
                if let verum_ast::LiteralKind::Int(intlit) = &lit.kind {
                    Some(intlit.value as i64)
                } else {
                    None
                }
            }
            ExprKind::Unary {
                op: verum_ast::UnOp::Neg,
                expr: inner,
            } => {
                if let ExprKind::Literal(lit) = &inner.kind {
                    if let verum_ast::LiteralKind::Int(intlit) = &lit.kind {
                        return Some(-(intlit.value as i64));
                    }
                }
                None
            }
            _ => None,
        }
    }
    let _ = walk(pred, &mut lo, &mut hi);
    if lo <= hi && (lo != i64::MIN || hi != i64::MAX) {
        Some((lo, hi))
    } else {
        None
    }
}

// --------------------------------------------------------------------
// Property discovery & runner
// --------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PropertyFunc {
    pub name: String,
    pub file: PathBuf,
    /// (param-name, Verum type) for each parameter in source order.
    pub params: Vec<(String, verum_ast::Type)>,
    /// `@property(runs = N)` override for this function.
    pub runs_override: Option<u32>,
    /// `@property(seed = 0x...)` override — single seed, no random sampling.
    pub seed_override: Option<Seed>,
}

/// Drive a single @property function for `runs` iterations (or once if
/// seed is pinned). Returns Ok on pass, Err with a descriptive message
/// containing shrunk counterexample on fail.
pub struct RunnerConfig {
    pub runs: u32,
    pub max_shrinks: u32,
    pub seed: Seed,
    pub pinned_seed: bool,
}

pub struct PropertyOutcome {
    pub iterations: u32,
    pub duration: Duration,
    pub failure: Option<PropertyFailure>,
}

pub struct PropertyFailure {
    pub seed: Seed,
    pub original_inputs: Vec<String>,
    pub shrunk_inputs: Vec<String>,
    pub shrink_steps: u32,
    pub message: String,
}

pub fn run_property(
    module: &Arc<VbcModule>,
    prop: &PropertyFunc,
    cfg: &RunnerConfig,
) -> PropertyOutcome {
    use verum_vbc::interpreter::Interpreter;
    let start = Instant::now();

    // Build a generator per parameter — always succeeds; unknown types
    // produce Generator::Fallback which emits a tracing::warn and yields
    // Value::unit() so the test runs even for partially-supported types.
    let gens: Vec<Generator> = prop
        .params
        .iter()
        .map(|(_, ty)| Generator::for_type(ty).unwrap_or(Generator::Fallback))
        .collect();
    if gens.iter().any(|g| matches!(g, Generator::Fallback)) {
        tracing::warn!(
            "property `{}`: one or more parameters use Generator::Fallback \
             (unknown type); add a Generator variant for accurate coverage",
            prop.name
        );
    }

    // Resolve VBC FunctionId by name.
    let fid: FunctionId = match module
        .functions
        .iter()
        .find(|f| module.get_string(f.name) == Some(prop.name.as_str()))
        .map(|f| f.id)
    {
        Some(id) => id,
        None => {
            return PropertyOutcome {
                iterations: 0,
                duration: start.elapsed(),
                failure: Some(PropertyFailure {
                    seed: cfg.seed,
                    original_inputs: vec![],
                    shrunk_inputs: vec![],
                    shrink_steps: 0,
                    message: format!("property `{}` not found in compiled VBC", prop.name),
                }),
            };
        }
    };

    let mut interp = Interpreter::new(Arc::clone(module));
    // Property bodies are typically short but the cumulative counter
    // kills us across many iterations. Disable both caps (see also
    // bench.rs comment for the same gate).
    interp.state.config.max_instructions = 0;
    interp.state.config.timeout_ms = 0;

    let total_runs = if cfg.pinned_seed { 1 } else { cfg.runs };

    for i in 0..total_runs {
        // Fresh generator streams per iteration, independent of other iters.
        let seed = if cfg.pinned_seed {
            cfg.seed
        } else {
            {
                // Mix seed + iteration so each run gets a distinct yet
                // deterministic starting point.
                let (a, _) = split(Seed(
                    cfg.seed.0 ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15),
                ));
                a
            }
        };

        // Derive per-parameter streams from the iteration seed.
        let (mut left, mut right) = split(seed);
        let mut inputs: Vec<TreeValue> = Vec::with_capacity(gens.len());
        for (idx, g) in gens.iter().enumerate() {
            let mut s = if idx == 0 { left } else { right };
            let v = g.sample(&mut s);
            // Alternate consumers so we never run out of independent splits.
            if idx % 2 == 0 {
                left = s;
            } else {
                right = s;
            }
            inputs.push(v);
        }

        // Invoke.
        let args: Vec<Value> = match inputs
            .iter()
            .map(|tv| tv.to_vbc_value(&mut interp))
            .collect::<Result<Vec<_>>>()
        {
            Ok(a) => a,
            Err(e) => {
                return PropertyOutcome {
                    iterations: i,
                    duration: start.elapsed(),
                    failure: Some(PropertyFailure {
                        seed,
                        original_inputs: inputs.iter().map(TreeValue::display).collect(),
                        shrunk_inputs: vec![],
                        shrink_steps: 0,
                        message: format!("host-side arg encode: {}", e),
                    }),
                };
            }
        };

        let outcome = call_with_args(&mut interp, fid, &args);
        if let Err(e) = outcome {
            // Shrink.
            let original = inputs.iter().map(TreeValue::display).collect();
            let (shrunk, steps) = shrink_failure(&mut interp, fid, inputs, cfg.max_shrinks);
            return PropertyOutcome {
                iterations: i + 1,
                duration: start.elapsed(),
                failure: Some(PropertyFailure {
                    seed,
                    original_inputs: original,
                    shrunk_inputs: shrunk,
                    shrink_steps: steps,
                    message: format!("{:?}", e),
                }),
            };
        }
    }

    PropertyOutcome {
        iterations: total_runs,
        duration: start.elapsed(),
        failure: None,
    }
}

/// Greedy shrinker: at each step, try shrinks of every input; keep the
/// first shrink that still fails; repeat until no shrinks fail or we
/// exhaust the budget. Classic QuickCheck strategy adapted for our
/// flat vector of inputs.
fn shrink_failure(
    interp: &mut verum_vbc::interpreter::Interpreter,
    fid: FunctionId,
    mut inputs: Vec<TreeValue>,
    budget: u32,
) -> (Vec<String>, u32) {
    let mut steps = 0u32;
    'outer: loop {
        if steps >= budget {
            break;
        }
        for idx in 0..inputs.len() {
            let candidates = inputs[idx].shrink();
            for cand in candidates {
                if steps >= budget {
                    break 'outer;
                }
                steps += 1;
                let mut trial = inputs.clone();
                trial[idx] = cand;
                let args: Vec<Value> = match trial
                    .iter()
                    .map(|tv| tv.to_vbc_value(interp))
                    .collect::<Result<Vec<_>>>()
                {
                    Ok(a) => a,
                    Err(_) => continue,
                };
                if call_with_args(interp, fid, &args).is_err() {
                    inputs = trial;
                    // Make progress on this index again from the top —
                    // smaller values often shrink further.
                    continue 'outer;
                }
            }
        }
        break;
    }
    (inputs.iter().map(TreeValue::display).collect(), steps)
}

// --------------------------------------------------------------------
// Helper: call a VBC function with arguments and run dispatch_loop_table
// --------------------------------------------------------------------
//

// `Interpreter::call` as published does NOT allocate register slots in
// the register file — only frames the call stack. Executing the body
// then tries to pop register ranges that were never allocated, which
// panics at `pop_frame(base)` with "Invalid frame base". This helper
// does the full setup: push both the call-stack frame and the register
// frame with `register_count` from the function descriptor, copy the
// args into registers [0..N), then run `dispatch_loop_table` (which
// does NOT re-push a frame — that's `execute_table`'s job).
pub fn call_parametrised(
    interp: &mut verum_vbc::interpreter::Interpreter,
    fid: FunctionId,
    args: &[Value],
) -> verum_vbc::interpreter::InterpreterResult<Value> {
    call_with_args(interp, fid, args)
}

fn call_with_args(
    interp: &mut verum_vbc::interpreter::Interpreter,
    fid: FunctionId,
    args: &[Value],
) -> verum_vbc::interpreter::InterpreterResult<Value> {
    use verum_vbc::instruction::Reg;
    use verum_vbc::interpreter::{InterpreterError, dispatch_loop_table};

    let func = interp
        .state
        .module
        .get_function(fid)
        .ok_or(InterpreterError::FunctionNotFound(fid))?;
    // Honour the function's declared register count, but always reserve
    // at least enough slots for the arguments we're passing in.
    let reg_count = func.register_count.max(args.len() as u16);
    let _ = interp
        .state
        .call_stack
        .push_frame(fid, reg_count, 0, Reg(0))?;
    let base = interp.state.registers.push_frame(reg_count);
    for (i, a) in args.iter().enumerate() {
        interp.state.registers.set(base, Reg(i as u16), *a);
    }
    dispatch_loop_table(&mut interp.state)
}

// --------------------------------------------------------------------
// Regression database — Hypothesis-style replay of failing seeds
// --------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RegressionEntry {
    pub test: String,
    pub seed: String,
    pub first_seen: String,
    pub shrunk_input: String,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RegressionDb {
    pub schema: String,
    pub entries: Vec<RegressionEntry>,
}

fn db_path() -> PathBuf {
    PathBuf::from("target/test/pbt-regressions.json")
}

pub fn load_regression_db() -> RegressionDb {
    match fs::read_to_string(db_path()) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => RegressionDb::default(),
    }
}

pub fn save_regression_db(db: &RegressionDb) -> Result<()> {
    let p = db_path();
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| CliError::Custom(format!("mkdir {}: {}", parent.display(), e)))?;
    }
    let json =
        serde_json::to_string_pretty(db).map_err(|e| CliError::Custom(format!("json: {}", e)))?;
    fs::write(&p, json).map_err(|e| CliError::Custom(format!("write: {}", e)))
}

pub fn seeds_for(db: &RegressionDb, test: &str) -> Vec<Seed> {
    db.entries
        .iter()
        .filter(|e| e.test == test)
        .filter_map(|e| Seed::from_hex(&e.seed))
        .collect()
}

pub fn record_regression(db: &mut RegressionDb, test: &str, seed: Seed, shrunk_input: &str) {
    let hex = seed.to_hex();
    // Don't duplicate.
    if db.entries.iter().any(|e| e.test == test && e.seed == hex) {
        return;
    }
    if db.schema.is_empty() {
        db.schema = "verum-pbt-regressions/v1".to_string();
    }
    db.entries.push(RegressionEntry {
        test: test.to_string(),
        seed: hex,
        first_seen: chrono::Utc::now().to_rfc3339(),
        shrunk_input: shrunk_input.to_string(),
    });
}

// --------------------------------------------------------------------
// Discovery helpers — called from test.rs
// --------------------------------------------------------------------

/// Walk an already-parsed module, picking out `@property` functions
/// (plus their parameter types for generator selection).
pub fn discover_properties_in_module(
    module: &verum_ast::Module,
    module_name: &str,
    file: &std::path::Path,
) -> Vec<PropertyFunc> {
    let mut out = Vec::new();
    for item in &module.items {
        if let ItemKind::Function(func) = &item.kind {
            if !func
                .attributes
                .iter()
                .any(|a| a.name.as_str() == "property")
            {
                continue;
            }
            let (runs, seed) = parse_property_attr_args(&func.attributes);
            let mut params = Vec::new();
            for p in &func.params {
                if let FunctionParamKind::Regular { pattern, ty, .. } = &p.kind {
                    let name = pattern_binding_name(pattern).unwrap_or_else(|| "_".to_string());
                    params.push((name, ty.clone()));
                }
            }
            out.push(PropertyFunc {
                name: func.name.to_string(),
                file: file.to_path_buf(),
                params,
                runs_override: runs,
                seed_override: seed,
            });
            let _ = module_name; // future: prefix into PropertyFunc::name for reporting
        }
    }
    out
}

fn pattern_binding_name(p: &verum_ast::Pattern) -> Option<String> {
    use verum_ast::PatternKind;
    match &p.kind {
        PatternKind::Ident { name, .. } => Some(name.to_string()),
        _ => None,
    }
}

fn parse_property_attr_args(attrs: &[verum_ast::Attribute]) -> (Option<u32>, Option<Seed>) {
    use verum_ast::ExprKind;
    let mut runs = None;
    let mut seed = None;
    for a in attrs {
        if a.name.as_str() != "property" {
            continue;
        }
        let args = match &a.args {
            verum_common::Maybe::Some(a) => a,
            _ => continue,
        };
        for e in args.iter() {
            if let ExprKind::NamedArg { name, value } = &e.kind {
                let key = name.to_string();
                if let ExprKind::Literal(lit) = &value.kind {
                    if let verum_ast::LiteralKind::Int(intlit) = &lit.kind {
                        let n = intlit.value;
                        match key.as_str() {
                            "runs" if n > 0 => {
                                runs = Some(n as u32);
                            }
                            "seed" => {
                                seed = Some(Seed(n as u64));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    (runs, seed)
}

// --------------------------------------------------------------------
// Tests
// --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shrink_int_toward_zero() {
        let s = shrink_int(100);
        assert!(s.contains(&0));
        assert!(s.iter().all(|x| x.abs() < 100));
    }

    #[test]
    fn shrink_int_zero_is_minimal() {
        assert!(shrink_int(0).is_empty());
    }

    #[test]
    fn shrink_text_drops_chars() {
        let s = shrink_text("abc");
        assert!(s.contains(&String::new()));
        assert!(s.iter().any(|x| x.len() == 2));
    }

    #[test]
    fn seed_roundtrip_hex() {
        let s = Seed(0xDEADBEEFCAFEBABE);
        assert_eq!(Seed::from_hex(&s.to_hex()).unwrap().0, s.0);
    }

    #[test]
    fn rng_is_deterministic() {
        let mut a = Seed(42);
        let mut b = Seed(42);
        let seq_a: Vec<u64> = (0..10).map(|_| next_u64(&mut a)).collect();
        let seq_b: Vec<u64> = (0..10).map(|_| next_u64(&mut b)).collect();
        assert_eq!(seq_a, seq_b);
    }

    #[test]
    fn int_gen_covers_edges() {
        let mut seen_zero = false;
        let mut seed = Seed(1);
        for _ in 0..200 {
            if let TreeValue::Int { value: 0, .. } = gen_int(&mut seed) {
                seen_zero = true;
                break;
            }
        }
        assert!(seen_zero, "int gen should hit 0 within 200 draws");
    }

    #[test]
    fn int_range_stays_in_bounds() {
        let mut seed = Seed(7);
        for _ in 0..500 {
            if let TreeValue::Int { value: n, .. } = gen_int_range(&mut seed, -5, 5) {
                assert!(n >= -5 && n <= 5, "got {} outside [-5,5]", n);
            }
        }
    }

    // ---- compound generator tests ----

    #[test]
    fn maybe_gen_produces_both_variants() {
        let g = Generator::Maybe(Box::new(Generator::Int));
        let mut seed = Seed(0xABCD);
        let mut saw_none = false;
        let mut saw_some = false;
        for _ in 0..100 {
            match g.sample(&mut seed) {
                TreeValue::Maybe(None) => saw_none = true,
                TreeValue::Maybe(Some(_)) => saw_some = true,
                _ => panic!("unexpected variant"),
            }
            if saw_none && saw_some {
                break;
            }
        }
        assert!(saw_none, "Maybe gen should produce None");
        assert!(saw_some, "Maybe gen should produce Some");
    }

    #[test]
    fn maybe_none_shrinks_to_nothing() {
        let v = TreeValue::Maybe(None);
        assert!(v.shrink().is_empty());
    }

    #[test]
    fn maybe_some_shrinks_toward_none() {
        let v = TreeValue::Maybe(Some(Box::new(TreeValue::Int { value: 42, lo: i64::MIN, hi: i64::MAX })));
        let shrinks = v.shrink();
        assert!(shrinks.contains(&TreeValue::Maybe(None)));
    }

    #[test]
    fn result_gen_produces_both_arms() {
        let g = Generator::Result {
            ok: Box::new(Generator::Int),
            err: Box::new(Generator::Bool),
        };
        let mut seed = Seed(0x1234);
        let mut saw_ok = false;
        let mut saw_err = false;
        for _ in 0..100 {
            match g.sample(&mut seed) {
                TreeValue::ResultVal { is_ok: true, .. } => saw_ok = true,
                TreeValue::ResultVal { is_ok: false, .. } => saw_err = true,
                _ => panic!("unexpected variant"),
            }
            if saw_ok && saw_err {
                break;
            }
        }
        assert!(saw_ok && saw_err);
    }

    #[test]
    fn list_gen_respects_max_len() {
        let g = Generator::List { elem: Box::new(Generator::Int), max_len: 5 };
        let mut seed = Seed(99);
        for _ in 0..200 {
            if let TreeValue::List(elems) = g.sample(&mut seed) {
                assert!(elems.len() <= 5, "list exceeded max_len");
            }
        }
    }

    #[test]
    fn list_empty_is_minimal_shrink() {
        let v = TreeValue::List(vec![
            TreeValue::Int { value: 1, lo: i64::MIN, hi: i64::MAX },
            TreeValue::Int { value: 2, lo: i64::MIN, hi: i64::MAX },
        ]);
        let shrinks = v.shrink();
        assert!(shrinks.iter().any(|s| matches!(s, TreeValue::List(elems) if elems.is_empty())));
    }

    #[test]
    fn ordering_gen_covers_all_three() {
        let g = Generator::Ordering;
        let mut seed = Seed(0xF00D);
        let mut seen = [false; 3];
        for _ in 0..200 {
            if let TreeValue::Ordering(v) = g.sample(&mut seed) {
                match v {
                    -1 => seen[0] = true,
                    0 => seen[1] = true,
                    1 => seen[2] = true,
                    _ => {}
                }
            }
        }
        assert!(seen.iter().all(|&x| x), "Ordering gen must cover Less/Equal/Greater");
    }

    #[test]
    fn ordering_equal_is_minimal() {
        assert!(TreeValue::Ordering(0).shrink().is_empty());
        assert_eq!(TreeValue::Ordering(-1).shrink(), vec![TreeValue::Ordering(0)]);
        assert_eq!(TreeValue::Ordering(1).shrink(), vec![TreeValue::Ordering(0)]);
    }

    #[test]
    fn tuple_gen_has_correct_arity() {
        let g = Generator::Tuple(vec![Generator::Int, Generator::Bool, Generator::Float]);
        let mut seed = Seed(42);
        if let TreeValue::Tuple(fields) = g.sample(&mut seed) {
            assert_eq!(fields.len(), 3);
        } else {
            panic!("expected Tuple");
        }
    }

    #[test]
    fn unit_tuple_gen_is_empty_tuple() {
        let g = Generator::Tuple(vec![]);
        let mut seed = Seed(0);
        if let TreeValue::Tuple(fields) = g.sample(&mut seed) {
            assert!(fields.is_empty());
        } else {
            panic!("expected empty Tuple");
        }
    }

    #[test]
    fn for_type_inner_strips_reference() {
        use verum_ast::Span;
        let inner = verum_ast::Type::int(Span::default());
        let ref_ty = verum_ast::Type::new(
            TypeKind::Reference { mutable: false, inner: Box::new(inner).into() },
            Span::default(),
        );
        let g = Generator::for_type_inner(&ref_ty);
        assert!(matches!(g, Generator::Int));
    }

    #[test]
    fn display_compound_types() {
        assert_eq!(TreeValue::Maybe(None).display(), "None");
        assert_eq!(
            TreeValue::Maybe(Some(Box::new(TreeValue::Bool(true)))).display(),
            "Some(true)"
        );
        assert_eq!(TreeValue::Ordering(-1).display(), "Less");
        assert_eq!(TreeValue::Ordering(0).display(), "Equal");
        assert_eq!(TreeValue::Ordering(1).display(), "Greater");
        assert_eq!(TreeValue::Tuple(vec![]).display(), "()");
        assert_eq!(
            TreeValue::List(vec![
                TreeValue::Int { value: 1, lo: i64::MIN, hi: i64::MAX },
            ])
            .display(),
            "[1]"
        );
    }
}
