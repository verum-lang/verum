//! Well-known Verum stdlib type names.
//!

//! Centralizes the string constants used throughout the compiler to identify
//! stdlib types (List, Map, Text, Channel, etc.), replacing hundreds of scattered
//! string literals with a single enum.
//!

//! This module lives in `verum_common` so all compiler crates can use it without
//! cross-crate dependency issues.

/// Well-known Verum standard library types referenced during compilation.
///

/// These are the types that the compiler needs special handling for — collection
/// types, wrapper types, concurrency primitives, etc. Using this enum instead of
/// raw string comparisons prevents typos and makes the set of special types explicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WellKnownType {
    // Primitives
    Int,
    Float,
    Bool,

    // Text
    Text,
    Char,

    // Collections
    List,
    Map,
    Set,
    Deque,
    BTreeMap,
    BTreeSet,
    BinaryHeap,

    // Wrappers
    Maybe,
    Result,
    Heap,
    Shared,

    // Concurrency
    Channel,
    Mutex,
    Task,
    Nursery,
    Semaphore,
    RwLock,
    Barrier,
    WaitGroup,
    Once,
    AtomicInt,
    AtomicBool,

    // Time
    Duration,
    Instant,

    // Misc
    Ordering,
    Range,
}

impl WellKnownType {
    /// The canonical string name for this type.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Int => "Int",
            Self::Float => "Float",
            Self::Bool => "Bool",
            Self::Text => "Text",
            Self::Char => "Char",
            Self::List => "List",
            Self::Map => "Map",
            Self::Set => "Set",
            Self::Deque => "Deque",
            Self::BTreeMap => "BTreeMap",
            Self::BTreeSet => "BTreeSet",
            Self::BinaryHeap => "BinaryHeap",
            Self::Maybe => "Maybe",
            Self::Result => "Result",
            Self::Heap => "Heap",
            Self::Shared => "Shared",
            Self::Channel => "Channel",
            Self::Mutex => "Mutex",
            Self::Task => "Task",
            Self::Nursery => "Nursery",
            Self::Semaphore => "Semaphore",
            Self::RwLock => "RwLock",
            Self::Barrier => "Barrier",
            Self::WaitGroup => "WaitGroup",
            Self::Once => "Once",
            Self::AtomicInt => "AtomicInt",
            Self::AtomicBool => "AtomicBool",
            Self::Duration => "Duration",
            Self::Instant => "Instant",
            Self::Ordering => "Ordering",
            Self::Range => "Range",
        }
    }

    /// Try to resolve a string name to a well-known type.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "Int" => Some(Self::Int),
            "Float" => Some(Self::Float),
            "Bool" => Some(Self::Bool),
            "Text" => Some(Self::Text),
            "Char" => Some(Self::Char),
            "List" => Some(Self::List),
            "Map" => Some(Self::Map),
            "Set" => Some(Self::Set),
            "Deque" => Some(Self::Deque),
            "BTreeMap" => Some(Self::BTreeMap),
            "BTreeSet" => Some(Self::BTreeSet),
            "BinaryHeap" => Some(Self::BinaryHeap),
            "Maybe" => Some(Self::Maybe),
            "Result" => Some(Self::Result),
            "Heap" => Some(Self::Heap),
            "Shared" => Some(Self::Shared),
            "Channel" => Some(Self::Channel),
            "Mutex" => Some(Self::Mutex),
            "Task" => Some(Self::Task),
            "Nursery" => Some(Self::Nursery),
            "Semaphore" => Some(Self::Semaphore),
            "RwLock" => Some(Self::RwLock),
            "Barrier" => Some(Self::Barrier),
            "WaitGroup" => Some(Self::WaitGroup),
            "Once" => Some(Self::Once),
            "AtomicInt" => Some(Self::AtomicInt),
            "AtomicBool" => Some(Self::AtomicBool),
            "Duration" => Some(Self::Duration),
            "Instant" => Some(Self::Instant),
            "Ordering" => Some(Self::Ordering),
            "Range" => Some(Self::Range),
            _ => None,
        }
    }

    /// Check if a string name matches this well-known type.
    pub fn matches(self, name: &str) -> bool {
        name == self.as_str()
    }

    /// Check if this type is a collection (List, Map, Set, Deque, BTreeMap, BTreeSet, BinaryHeap).
    pub const fn is_collection(self) -> bool {
        matches!(
            self,
            Self::List
                | Self::Map
                | Self::Set
                | Self::Deque
                | Self::BTreeMap
                | Self::BTreeSet
                | Self::BinaryHeap
        )
    }

    /// Check if this type is a concurrency primitive.
    pub const fn is_concurrency(self) -> bool {
        matches!(
            self,
            Self::Channel
                | Self::Mutex
                | Self::Task
                | Self::Nursery
                | Self::Semaphore
                | Self::RwLock
                | Self::Barrier
                | Self::WaitGroup
                | Self::Once
                | Self::AtomicInt
                | Self::AtomicBool
        )
    }

    /// Check if this type is a primitive (Int, Float, Bool).
    pub const fn is_primitive(self) -> bool {
        matches!(self, Self::Int | Self::Float | Self::Bool)
    }

    /// Check if this type is a wrapper (Maybe, Result, Heap, Shared).
    pub const fn is_wrapper(self) -> bool {
        matches!(self, Self::Maybe | Self::Result | Self::Heap | Self::Shared)
    }

    /// Check if this type is a smart pointer (Heap, Shared).
    /// Both wrap a single `T` and are auto-deref'd for method resolution.
    pub const fn is_smart_pointer(self) -> bool {
        matches!(self, Self::Heap | Self::Shared)
    }

    /// Check if the given string names a smart-pointer type (Heap or Shared).
    pub fn is_smart_pointer_name(name: &str) -> bool {
        Self::from_name(name).is_some_and(|w| w.is_smart_pointer())
    }

    /// Check if the given name is any well-known type.
    pub fn is_well_known(name: &str) -> bool {
        Self::from_name(name).is_some()
    }

    /// Returns a non-zero type hint for the Len instruction if this type supports
    /// built-in length queries, or 0 if it does not.
    /// These hints correspond to the interpreter's Len opcode dispatch.
    pub const fn len_type_hint(self) -> u8 {
        match self {
            Self::List => 1,
            Self::Map => 2,
            Self::Set => 3,
            Self::Deque => 4,
            Self::Text => 5,
            Self::Channel => 6,
            _ => 0,
        }
    }

    /// Returns true if this type's `.len()` must use the built-in Len opcode
    /// rather than a compiled method (because compiled stdlib .len() uses GetF
    /// offsets that don't match the runtime object layout).
    pub const fn requires_builtin_len(self) -> bool {
        matches!(self, Self::Text | Self::List | Self::Map)
    }

    /// Returns true if this type is a well-known type that has built-in method
    /// dispatch in the interpreter (primitives, collections, wrappers, etc.).
    pub fn has_builtin_dispatch(name: &str) -> bool {
        Self::from_name(name).is_some()
    }
}

/// Well-known variant constructor tags used by stdlib sum types.
///

/// These are the constructor names that the compiler may need to recognize
/// when doing pattern matching or value construction in the meta system.
/// Centralizes strings like "Some", "None", "Ok", "Err" that were previously
/// scattered across the compiler.
pub mod variant_tags {
    /// Maybe<T> constructors
    pub const SOME: &str = "Some";
    pub const NONE: &str = "None";

    /// Result<T, E> constructors
    pub const OK: &str = "Ok";
    pub const ERR: &str = "Err";

    /// Haskell-style aliases sometimes seen in proofs
    pub const JUST: &str = "Just";
    pub const NOTHING: &str = "Nothing";

    /// Check if a name is any well-known Maybe/Option constructor.
    pub fn is_maybe_constructor(name: &str) -> bool {
        matches!(name, SOME | NONE | JUST | NOTHING)
    }

    /// Check if a name is any well-known Result constructor.
    pub fn is_result_constructor(name: &str) -> bool {
        matches!(name, OK | ERR)
    }

    /// Null-like sentinel values recognized during serialization / kernel dispatch.
    pub fn is_null_sentinel(name: &str) -> bool {
        matches!(name, "null" | NONE | "nil")
    }
}

/// Canonical layout of the variants of `core::base::maybe::Maybe<T>`.
///
/// Source-of-truth: `core/base/maybe.vr`:
/// ```text
///     public type Maybe<T> is None | Some(T);
/// ```
/// Tags follow declaration order: `None = 0`, `Some = 1`.
///
/// **Drift contract:** any reorder in the .vr file MUST be reflected here,
/// and the matrix-pinning test in `tests::maybe_variant_layout_pinned`
/// catches the divergence at test time.
pub const MAYBE_VARIANT_LAYOUT: &[(&str, u32)] = &[
    ("None", 0),
    ("Some", 1),
];

/// Canonical layout of the variants of `core::base::result::Result<T, E>`.
///
/// Source-of-truth: `core/base/result.vr`:
/// ```text
///     public type Result<T, E> is Ok(T) | Err(E);
/// ```
/// Tags follow declaration order: `Ok = 0`, `Err = 1`.
pub const RESULT_VARIANT_LAYOUT: &[(&str, u32)] = &[
    ("Ok", 0),
    ("Err", 1),
];

/// Canonical layout of the variants of `core::base::ordering::Ordering`.
///
/// **Single source of truth.** Both VBC codegen (the builtin variant registry
/// in `verum_vbc/src/codegen/mod.rs`) and the runtime constructor (`make_ordering`
/// in `verum_vbc/src/interpreter/dispatch_table/handlers/method_dispatch.rs`)
/// consult this constant — neither hardcodes its own copy of the variant
/// order. If anybody edits the source-of-truth `core/base/ordering.vr` to
/// declare the variants in a different order without updating this constant
/// (or vice versa), the load-time validator (`ordering_layout::validate`)
/// catches the drift and refuses to load the module.
///
/// **Drift contract:** the slice's order MUST match the order in
/// `core/base/ordering.vr`:
/// ```text
///     public type Ordering is Less | Equal | Greater;
/// ```
/// — which produces variant tags 0, 1, 2 in declaration order.
pub const ORDERING_VARIANT_LAYOUT: &[(&str, u32)] = &[
    ("Less", 0),
    ("Equal", 1),
    ("Greater", 2),
];

/// Canonical layout of the variants of `core::base::data::Data`.
///
/// **Single source of truth.** Any VBC codegen or runtime code that constructs
/// or pattern-matches `Data` values must read tag numbers from this constant
/// rather than hardcoding them.
///
/// **Drift contract:** the slice's order MUST match `core/base/data.vr`:
/// ```text
///     public type Data is
///         | Null
///         | Bool(Bool)
///         | Int(Int)
///         | Float(Float)
///         | Text(Text)
///         | Array(List<Data>)
///         | Object(Map<Text, Data>);
/// ```
/// — which produces tags 0–6 in declaration order.
pub const DATA_VARIANT_LAYOUT: &[(&str, u32)] = &[
    ("Null", 0),
    ("Bool", 1),
    ("Int", 2),
    ("Float", 3),
    ("Text", 4),
    ("Array", 5),
    ("Object", 6),
];

/// Canonical layout of the variants of `core::base::ops::ControlFlow<B, C>`.
///
/// **Single source of truth.** `compile_try` in VBC codegen reads this constant
/// to interpret the result of `Try::branch()` without hardcoding tag values.
///
/// **Drift contract:** the slice's order MUST match `core/base/ops.vr`:
/// ```text
///     public type ControlFlow<B, C> is Continue(C) | Break(B);
/// ```
/// — `Continue = 0`, `Break = 1` in declaration order.
pub const CONTROLFLOW_VARIANT_LAYOUT: &[(&str, u32)] = &[
    ("Continue", 0),
    ("Break", 1),
];

/// Canonical layout of the variants of `core::base::env::VarError`.
///
/// **Single source of truth.** The runtime intercept for `env::var`
/// (`env_runtime.rs`) must use this constant when constructing
/// `VarError.NotPresent` or `VarError.NotUnicode(bytes)` variants via
/// `wrap_in_variant` — never the raw integer literals `0` / `1`.
///
/// **Drift contract:** the slice's order MUST match `core/base/env.vr`:
/// ```text
///     public type VarError is
///         | NotPresent
///         | NotUnicode(List<Byte>);
/// ```
/// — `NotPresent = 0`, `NotUnicode = 1` in declaration order.
pub const VARERROR_VARIANT_LAYOUT: &[(&str, u32)] = &[
    ("NotPresent", 0),
    ("NotUnicode", 1),
];

/// Returns the canonical tag for `VarError::NotPresent` (tag 0).
///
/// Use this instead of the literal `0` in `wrap_in_variant(state, "VarError", ...)`.
pub fn varerror_not_present_tag() -> u32 {
    VARERROR_VARIANT_LAYOUT
        .iter()
        .find_map(|&(n, t)| if n == "NotPresent" { Some(t) } else { None })
        .expect("VARERROR_VARIANT_LAYOUT must contain 'NotPresent'")
}

/// Returns the canonical tag for `VarError::NotUnicode` (tag 1).
///
/// Use this instead of the literal `1` in `wrap_in_variant(state, "VarError", ...)`.
pub fn varerror_not_unicode_tag() -> u32 {
    VARERROR_VARIANT_LAYOUT
        .iter()
        .find_map(|&(n, t)| if n == "NotUnicode" { Some(t) } else { None })
        .expect("VARERROR_VARIANT_LAYOUT must contain 'NotUnicode'")
}

/// Returns the canonical success variant tag for `Maybe<T>` (the tag for `Some`).
///
/// Derived from `MAYBE_VARIANT_LAYOUT` — same source of truth used by
/// `compile_try`, MakeVariant, and @property generators.
///
/// # Panics
/// Panics if `MAYBE_VARIANT_LAYOUT` is missing the `"Some"` entry, which
/// would indicate an inconsistency between the constant and `core/base/maybe.vr`.
pub fn maybe_success_tag() -> u32 {
    MAYBE_VARIANT_LAYOUT
        .iter()
        .find_map(|&(n, t)| if n == "Some" { Some(t) } else { None })
        .expect("MAYBE_VARIANT_LAYOUT must contain 'Some'")
}

/// Returns the canonical success variant tag for `Result<T, E>` (the tag for `Ok`).
///
/// Derived from `RESULT_VARIANT_LAYOUT` — same source of truth used by
/// `compile_try`, MakeVariant, and @property generators.
///
/// # Panics
/// Panics if `RESULT_VARIANT_LAYOUT` is missing the `"Ok"` entry.
pub fn result_success_tag() -> u32 {
    RESULT_VARIANT_LAYOUT
        .iter()
        .find_map(|&(n, t)| if n == "Ok" { Some(t) } else { None })
        .expect("RESULT_VARIANT_LAYOUT must contain 'Ok'")
}

/// Look up the canonical Verum tag for a Rust `std::cmp::Ordering` value.
///
/// Translates `std::cmp::Ordering` → variant name → tag from the canonical
/// layout. Panics with a structured message if the layout is missing a
/// well-known variant — that would indicate `ORDERING_VARIANT_LAYOUT` was
/// edited in a way that drops a variant, which is a programming error
/// rather than a recoverable runtime condition.
pub fn ordering_tag_for_std(ord: std::cmp::Ordering) -> u32 {
    let name = match ord {
        std::cmp::Ordering::Less => "Less",
        std::cmp::Ordering::Equal => "Equal",
        std::cmp::Ordering::Greater => "Greater",
    };
    ORDERING_VARIANT_LAYOUT
        .iter()
        .find_map(|(n, t)| if *n == name { Some(*t) } else { None })
        .unwrap_or_else(|| {
            panic!(
                "ORDERING_VARIANT_LAYOUT is missing variant `{}` — \
                 check core/base/ordering.vr and the layout constant in \
                 verum_common/src/well_known_types.rs",
                name
            )
        })
}

#[cfg(test)]
mod ordering_layout_tests {
    use super::*;

    #[test]
    fn layout_pins_canonical_three_variants() {
        // Three variants, in canonical order. If this asserts, either the .vr
        // file changed and the constant must follow, or vice versa — but the
        // load-time validator will already have refused to load.
        assert_eq!(ORDERING_VARIANT_LAYOUT.len(), 3);
        assert_eq!(ORDERING_VARIANT_LAYOUT[0], ("Less", 0));
        assert_eq!(ORDERING_VARIANT_LAYOUT[1], ("Equal", 1));
        assert_eq!(ORDERING_VARIANT_LAYOUT[2], ("Greater", 2));
    }

    #[test]
    fn ordering_tag_for_std_matches_layout() {
        assert_eq!(ordering_tag_for_std(std::cmp::Ordering::Less), 0);
        assert_eq!(ordering_tag_for_std(std::cmp::Ordering::Equal), 1);
        assert_eq!(ordering_tag_for_std(std::cmp::Ordering::Greater), 2);
    }

    /// Pins the canonical layout of `Maybe<T>`. Mirrors the
    /// Ordering pattern: any change to the variant order in
    /// `core/base/maybe.vr` must be reflected here, and vice versa.
    /// Codegen builtin variant registration consults this constant.
    #[test]
    fn maybe_variant_layout_pinned() {
        assert_eq!(MAYBE_VARIANT_LAYOUT.len(), 2);
        assert_eq!(MAYBE_VARIANT_LAYOUT[0], ("None", 0));
        assert_eq!(MAYBE_VARIANT_LAYOUT[1], ("Some", 1));
    }

    /// Pins the canonical layout of `Result<T, E>`.
    #[test]
    fn result_variant_layout_pinned() {
        assert_eq!(RESULT_VARIANT_LAYOUT.len(), 2);
        assert_eq!(RESULT_VARIANT_LAYOUT[0], ("Ok", 0));
        assert_eq!(RESULT_VARIANT_LAYOUT[1], ("Err", 1));
    }

    /// Pins `Data` variant order: Null=0, Bool=1, Int=2, Float=3, Text=4, Array=5, Object=6.
    #[test]
    fn data_variant_layout_pinned() {
        assert_eq!(DATA_VARIANT_LAYOUT.len(), 7);
        assert_eq!(DATA_VARIANT_LAYOUT[0], ("Null", 0));
        assert_eq!(DATA_VARIANT_LAYOUT[1], ("Bool", 1));
        assert_eq!(DATA_VARIANT_LAYOUT[2], ("Int", 2));
        assert_eq!(DATA_VARIANT_LAYOUT[3], ("Float", 3));
        assert_eq!(DATA_VARIANT_LAYOUT[4], ("Text", 4));
        assert_eq!(DATA_VARIANT_LAYOUT[5], ("Array", 5));
        assert_eq!(DATA_VARIANT_LAYOUT[6], ("Object", 6));
    }

    /// Pins `ControlFlow<B,C>` variant order: Continue=0, Break=1.
    #[test]
    fn controlflow_variant_layout_pinned() {
        assert_eq!(CONTROLFLOW_VARIANT_LAYOUT.len(), 2);
        assert_eq!(CONTROLFLOW_VARIANT_LAYOUT[0], ("Continue", 0));
        assert_eq!(CONTROLFLOW_VARIANT_LAYOUT[1], ("Break", 1));
    }

    /// `maybe_success_tag()` must return the tag for `Some` (not `None`).
    #[test]
    fn maybe_success_tag_is_some() {
        assert_eq!(maybe_success_tag(), 1, "Some is tag 1 per MAYBE_VARIANT_LAYOUT");
    }

    /// `result_success_tag()` must return the tag for `Ok` (not `Err`).
    #[test]
    fn result_success_tag_is_ok() {
        assert_eq!(result_success_tag(), 0, "Ok is tag 0 per RESULT_VARIANT_LAYOUT");
    }

    /// Cross-check: success tags are derived from the layout constants, not hardcoded.
    #[test]
    fn success_tags_consistent_with_layouts() {
        let maybe_some_tag = MAYBE_VARIANT_LAYOUT
            .iter()
            .find_map(|&(n, t)| if n == "Some" { Some(t) } else { None })
            .unwrap();
        assert_eq!(maybe_success_tag(), maybe_some_tag);

        let result_ok_tag = RESULT_VARIANT_LAYOUT
            .iter()
            .find_map(|&(n, t)| if n == "Ok" { Some(t) } else { None })
            .unwrap();
        assert_eq!(result_success_tag(), result_ok_tag);
    }

    // =========================================================================
    // Task #37 — Operator fast-path drift validator
    //
    // The `?`-operator fast path in `compile_try` (verum_vbc/codegen/expressions.rs)
    // emits `IsVar { tag: success_tag }` directly on the Maybe/Result value instead
    // of calling `Try::branch()` and then checking ControlFlow::Continue.
    //
    // This shortcut is correct ONLY when the success tag in MAYBE/RESULT_VARIANT_LAYOUT
    // corresponds to the variant that `branch()` maps to Continue.
    //
    // The invariants (documented as constants in core/base/maybe.vr + result.vr):
    //   Maybe::branch(): Some(v) → Continue(v), None → Break(None)
    //   Result::branch(): Ok(v) → Continue(v), Err(e) → Break(Err(e))
    //
    // The assertions below pin the contracts that make the fast path safe.
    // =========================================================================

    /// The ControlFlow::Continue tag must be distinct from ControlFlow::Break tag.
    /// The fast path exploits this to substitute a direct variant check for branch().
    #[test]
    fn operator_fastpath_drift_controlflow_tags_distinct() {
        let continue_tag = CONTROLFLOW_VARIANT_LAYOUT
            .iter()
            .find_map(|&(n, t)| if n == "Continue" { Some(t) } else { None })
            .expect("CONTROLFLOW_VARIANT_LAYOUT must contain 'Continue'");
        let break_tag = CONTROLFLOW_VARIANT_LAYOUT
            .iter()
            .find_map(|&(n, t)| if n == "Break" { Some(t) } else { None })
            .expect("CONTROLFLOW_VARIANT_LAYOUT must contain 'Break'");
        assert_ne!(
            continue_tag, break_tag,
            "Continue and Break must have different tags for the fast-path substitution to be valid",
        );
    }

    /// The Maybe success-tag (Some=1) and failure-tag (None=0) must be different.
    /// The fast path does `IsVar { tag: maybe_success_tag() }` — it is only correct
    /// if the success tag uniquely identifies the success variant.
    #[test]
    fn operator_fastpath_drift_maybe_tags_distinct() {
        let none_tag = MAYBE_VARIANT_LAYOUT
            .iter()
            .find_map(|&(n, t)| if n == "None" { Some(t) } else { None })
            .expect("MAYBE_VARIANT_LAYOUT must contain 'None'");
        assert_ne!(
            maybe_success_tag(),
            none_tag,
            "maybe_success_tag (Some) must differ from None tag; fast path would always Ret on None",
        );
    }

    /// The Result success-tag (Ok=0) and failure-tag (Err=1) must be different.
    #[test]
    fn operator_fastpath_drift_result_tags_distinct() {
        let err_tag = RESULT_VARIANT_LAYOUT
            .iter()
            .find_map(|&(n, t)| if n == "Err" { Some(t) } else { None })
            .expect("RESULT_VARIANT_LAYOUT must contain 'Err'");
        assert_ne!(
            result_success_tag(),
            err_tag,
            "result_success_tag (Ok) must differ from Err tag; fast path would always Ret on Err",
        );
    }

    /// All canonical layout constants must be internally consistent: unique tags
    /// and unique variant names within each constant.
    #[test]
    fn operator_fastpath_drift_all_layouts_well_formed() {
        let two_variant = &[
            (MAYBE_VARIANT_LAYOUT, "MAYBE", 2usize),
            (RESULT_VARIANT_LAYOUT, "RESULT", 2),
            (CONTROLFLOW_VARIANT_LAYOUT, "CONTROLFLOW", 2),
            (VARERROR_VARIANT_LAYOUT, "VARERROR", 2),
        ];
        let n_variant: &[(&[(&str, u32)], &str, usize)] =
            &[(DATA_VARIANT_LAYOUT, "DATA", 7)];

        for &(layout, name, expected_len) in two_variant.iter().chain(n_variant.iter()) {
            assert_eq!(layout.len(), expected_len, "{} layout must have {} variants", name, expected_len);
            let tags: std::collections::HashSet<u32> = layout.iter().map(|&(_, t)| t).collect();
            assert_eq!(tags.len(), expected_len, "{} layout must have unique tags", name);
            let names: std::collections::HashSet<&str> = layout.iter().map(|&(n, _)| n).collect();
            assert_eq!(names.len(), expected_len, "{} layout must have unique variant names", name);
        }
    }

    /// Pins `VarError` variant order: NotPresent=0, NotUnicode=1.
    /// The runtime env intercept (`env_runtime.rs`) must use these tags.
    #[test]
    fn varerror_variant_layout_pinned() {
        assert_eq!(VARERROR_VARIANT_LAYOUT.len(), 2);
        assert_eq!(VARERROR_VARIANT_LAYOUT[0], ("NotPresent", 0));
        assert_eq!(VARERROR_VARIANT_LAYOUT[1], ("NotUnicode", 1));
    }

    #[test]
    fn varerror_tag_helpers_consistent_with_layout() {
        assert_eq!(varerror_not_present_tag(), 0);
        assert_eq!(varerror_not_unicode_tag(), 1);
        assert_ne!(varerror_not_present_tag(), varerror_not_unicode_tag());
    }
}

/// Convenience constants for the most commonly referenced type names.
pub mod type_names {
    // Primitives
    pub const INT: &str = "Int";
    pub const FLOAT: &str = "Float";
    pub const BOOL: &str = "Bool";
    pub const TEXT: &str = "Text";
    pub const CHAR: &str = "Char";
    pub const BYTE: &str = "Byte";
    pub const UNIT: &str = "Unit";
    pub const NEVER: &str = "Never";

    // Integer variants
    pub const INT8: &str = "Int8";
    pub const INT16: &str = "Int16";
    pub const INT32: &str = "Int32";
    pub const INT64: &str = "Int64";
    pub const INT128: &str = "Int128";
    pub const INTSIZE: &str = "IntSize";
    pub const UINT8: &str = "UInt8";
    pub const UINT16: &str = "UInt16";
    pub const UINT32: &str = "UInt32";
    pub const UINT64: &str = "UInt64";
    pub const UINT128: &str = "UInt128";
    pub const USIZE: &str = "USize";

    // Float variants
    pub const FLOAT32: &str = "Float32";
    pub const FLOAT64: &str = "Float64";

    // Collections
    pub const LIST: &str = "List";
    pub const MAP: &str = "Map";
    pub const SET: &str = "Set";
    pub const DEQUE: &str = "Deque";
    pub const ARRAY: &str = "Array";
    pub const RANGE: &str = "Range";

    // Wrappers
    pub const MAYBE: &str = "Maybe";
    pub const RESULT: &str = "Result";
    pub const HEAP: &str = "Heap";
    pub const SHARED: &str = "Shared";

    // Concurrency
    pub const CHANNEL: &str = "Channel";
    pub const MUTEX: &str = "Mutex";
    pub const TASK: &str = "Task";
    pub const NURSERY: &str = "Nursery";
    pub const SEMAPHORE: &str = "Semaphore";
    pub const RWLOCK: &str = "RwLock";
    pub const BARRIER: &str = "Barrier";

    /// Returns true if `name` is a primitive numeric type (any Int or Float variant).
    pub fn is_numeric_type(name: &str) -> bool {
        is_integer_type(name) || is_float_type(name)
    }

    /// Returns true if `name` is any integer type variant (Int, Int8..Int128, UInt8..UInt128, etc.).
    pub fn is_integer_type(name: &str) -> bool {
        is_signed_integer_type(name) || is_unsigned_integer_type(name)
    }

    /// Returns true if `name` is a signed integer type.
    pub fn is_signed_integer_type(name: &str) -> bool {
        matches!(
            name,
            "Int"
                | "Int8"
                | "Int16"
                | "Int32"
                | "Int64"
                | "Int128"
                | "IntSize"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "isize"
        )
    }

    /// Returns true if `name` is an unsigned integer type.
    pub fn is_unsigned_integer_type(name: &str) -> bool {
        matches!(
            name,
            "UInt8"
                | "UInt16"
                | "UInt32"
                | "UInt64"
                | "UInt128"
                | "UIntSize"
                | "USize"
                | "Byte"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "usize"
        )
    }

    /// Returns true if `name` is any float type variant (Float, Float32, Float64).
    pub fn is_float_type(name: &str) -> bool {
        matches!(name, "Float" | "Float32" | "Float64" | "f32" | "f64")
    }

    /// Returns true if `name` is a primitive value type (no heap allocation needed).
    pub fn is_primitive_value_type(name: &str) -> bool {
        matches!(
            name,
            "Int"
                | "Float"
                | "Bool"
                | "Char"
                | "Byte"
                | "Unit"
                | "Int8"
                | "Int16"
                | "Int32"
                | "Int64"
                | "Int128"
                | "IntSize"
                | "UInt8"
                | "UInt16"
                | "UInt32"
                | "UInt64"
                | "UInt128"
                | "USize"
                | "Float32"
                | "Float64"
        )
    }

    /// Returns true if `name` is a collection type that supports `.len()` and iteration.
    pub fn is_collection_type(name: &str) -> bool {
        matches!(
            name,
            "List"
                | "Map"
                | "Set"
                | "Deque"
                | "Array"
                | "Range"
                | "BTreeMap"
                | "BTreeSet"
                | "BinaryHeap"
        )
    }

    /// Returns true if `name` is a type that supports built-in method dispatch
    /// (collections, wrappers, Text, etc.).
    pub fn is_builtin_method_type(name: &str) -> bool {
        matches!(
            name,
            "List"
                | "Map"
                | "Set"
                | "Deque"
                | "Channel"
                | "Text"
                | "Maybe"
                | "Result"
                | "Heap"
                | "Shared"
                | "Array"
                | "Range"
        )
    }

    /// Normalize a numeric type category: returns "Int" for any integer type,
    /// "Float" for any float type, or the name itself for non-numeric types.
    pub fn numeric_category(name: &str) -> &str {
        if is_integer_type(name) {
            INT
        } else if is_float_type(name) {
            FLOAT
        } else {
            name
        }
    }

    /// Returns the bit width of a numeric type, or None if not a fixed-width numeric.
    pub fn numeric_bit_width(name: &str) -> Option<u32> {
        match name {
            "Int8" | "UInt8" | "Byte" | "i8" | "u8" | "Bool" => Some(8),
            "Int16" | "UInt16" | "i16" | "u16" => Some(16),
            "Int32" | "UInt32" | "i32" | "u32" | "Float32" | "f32" => Some(32),
            "Int" | "Int64" | "UInt64" | "i64" | "u64" | "Float" | "Float64" | "f64" => Some(64),
            "Int128" | "UInt128" | "i128" | "u128" => Some(128),
            _ => None,
        }
    }

    /// Strip generic arguments from a type name: "List<Int>" -> "List", "Map<K, V>" -> "Map".
    pub fn strip_generic_args(name: &str) -> &str {
        match name.find('<') {
            Some(idx) => &name[..idx],
            None => name,
        }
    }
}

// =============================================================================
// Well-Known Protocols
// =============================================================================

/// Well-known Verum protocols that the compiler may need special handling for.
///

/// This centralizes protocol name strings, replacing scattered hardcoded comparisons
/// like `"Clone"`, `"Eq"`, `"Hash"` across the compiler. The compiler still needs to
/// know about these protocols for codegen (e.g., vtable layout, dynamic dispatch),
/// but all knowledge is centralized here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WellKnownProtocol {
    Copy,
    Clone,
    Eq,
    Ord,
    Hash,
    Default,
    Debug,
    Display,
    Drop,
    From,
    Into,
    Iterator,
    IntoIterator,
    Future,
    Stream,
    Error,
    Send,
    Sync,
    Write,
    Read,
    // Verum-specific protocol aliases (used in some codegen paths)
    Drawable,
    Printable,
    Hashable,
    Comparable,
}

impl WellKnownProtocol {
    /// The canonical string name for this protocol.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Copy => "Copy",
            Self::Clone => "Clone",
            Self::Eq => "Eq",
            Self::Ord => "Ord",
            Self::Hash => "Hash",
            Self::Default => "Default",
            Self::Debug => "Debug",
            Self::Display => "Display",
            Self::Drop => "Drop",
            Self::From => "From",
            Self::Into => "Into",
            Self::Iterator => "Iterator",
            Self::IntoIterator => "IntoIterator",
            Self::Future => "Future",
            Self::Stream => "Stream",
            Self::Error => "Error",
            Self::Send => "Send",
            Self::Sync => "Sync",
            Self::Write => "Write",
            Self::Read => "Read",
            Self::Drawable => "Drawable",
            Self::Printable => "Printable",
            Self::Hashable => "Hashable",
            Self::Comparable => "Comparable",
        }
    }

    /// Try to resolve a string name to a well-known protocol.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "Copy" => Some(Self::Copy),
            "Clone" => Some(Self::Clone),
            "Eq" => Some(Self::Eq),
            "Ord" => Some(Self::Ord),
            "Hash" => Some(Self::Hash),
            "Default" => Some(Self::Default),
            "Debug" => Some(Self::Debug),
            "Display" => Some(Self::Display),
            "Drop" => Some(Self::Drop),
            "From" => Some(Self::From),
            "Into" => Some(Self::Into),
            "Iterator" => Some(Self::Iterator),
            "IntoIterator" => Some(Self::IntoIterator),
            "Future" => Some(Self::Future),
            "Stream" => Some(Self::Stream),
            "Error" => Some(Self::Error),
            "Send" => Some(Self::Send),
            "Sync" => Some(Self::Sync),
            "Write" => Some(Self::Write),
            "Read" => Some(Self::Read),
            "Drawable" => Some(Self::Drawable),
            "Printable" => Some(Self::Printable),
            "Hashable" => Some(Self::Hashable),
            "Comparable" => Some(Self::Comparable),
            _ => None,
        }
    }

    /// Check if a string name matches this well-known protocol.
    pub fn matches(self, name: &str) -> bool {
        name == self.as_str()
    }

    /// Returns true if this protocol requires a fat reference (vtable pointer)
    /// when used as a dynamic dispatch target (protocol object / existential).
    ///

    /// This is the centralized definition of which protocols produce fat refs,
    /// replacing scattered `matches!()` lists throughout codegen.
    pub fn requires_fat_ref(self) -> bool {
        // All well-known protocols require fat refs when used as trait objects,
        // because dynamic dispatch needs a vtable pointer alongside the data pointer.
        true
    }

    /// Check if the given name is any well-known protocol that requires fat ref dispatch.
    pub fn is_fat_ref_protocol(name: &str) -> bool {
        Self::from_name(name).is_some_and(|p| p.requires_fat_ref())
    }
}

// =============================================================================
// Method-to-Protocol Mapping
// =============================================================================

/// Resolve a method name to its defining protocol (if the method is a well-known
/// protocol method).
///

/// This enables "dyn:Protocol.method" dispatch at the LLVM level when
/// monomorphization hasn't resolved the concrete type.
///

/// This centralizes the mapping that was previously hardcoded in
/// `verum_vbc/src/codegen/expressions.rs`.
pub fn method_to_protocol(method_name: &str) -> Option<WellKnownProtocol> {
    match method_name {
        "default" | "zero" => Some(WellKnownProtocol::Default),
        "hash" | "hash_value" => Some(WellKnownProtocol::Hash),
        "eq" | "ne" => Some(WellKnownProtocol::Eq),
        "cmp" | "lt" | "le" | "gt" | "ge" | "min" | "max" => Some(WellKnownProtocol::Ord),
        "clone" | "clone_from" => Some(WellKnownProtocol::Clone),
        "fmt" | "to_string" | "debug_string" => Some(WellKnownProtocol::Debug),
        "into_iter" | "iter" => Some(WellKnownProtocol::IntoIterator),
        "next" | "has_next" => Some(WellKnownProtocol::Iterator),
        "drop" => Some(WellKnownProtocol::Drop),
        "from" | "try_from" => Some(WellKnownProtocol::From),
        "into" | "try_into" => Some(WellKnownProtocol::Into),
        _ => None,
    }
}

// =============================================================================
// Primitive Protocol Implementations (Builtin Registry)
// =============================================================================

/// Check if a primitive type name implements a given protocol.
///

/// This centralizes the knowledge of which built-in/primitive types automatically
/// satisfy which protocols. Previously this was scattered across
/// `verum_types/src/specialization_selection.rs` in hardcoded match arms.
///

/// Note: This is intentionally hardcoded because primitive types are part of the
/// language definition, not the standard library. Their protocol implementations
/// cannot be discovered from source -- they are axioms of the type system.
pub fn primitive_implements_protocol(type_name: &str, protocol_name: &str) -> Option<bool> {
    let proto = WellKnownProtocol::from_name(protocol_name)?;

    let result = match type_name {
        // Int: Copy, Clone, Eq, Ord, Hash, Default
        "Int" => matches!(
            proto,
            WellKnownProtocol::Copy
                | WellKnownProtocol::Clone
                | WellKnownProtocol::Eq
                | WellKnownProtocol::Ord
                | WellKnownProtocol::Hash
                | WellKnownProtocol::Default
        ),
        // Float: Copy, Clone, Default (NOT Eq/Ord due to NaN)
        "Float" => matches!(
            proto,
            WellKnownProtocol::Copy | WellKnownProtocol::Clone | WellKnownProtocol::Default
        ),
        // Bool: Copy, Clone, Eq, Ord, Hash, Default
        "Bool" => matches!(
            proto,
            WellKnownProtocol::Copy
                | WellKnownProtocol::Clone
                | WellKnownProtocol::Eq
                | WellKnownProtocol::Ord
                | WellKnownProtocol::Hash
                | WellKnownProtocol::Default
        ),
        // Char: Copy, Clone, Eq, Ord, Hash
        "Char" => matches!(
            proto,
            WellKnownProtocol::Copy
                | WellKnownProtocol::Clone
                | WellKnownProtocol::Eq
                | WellKnownProtocol::Ord
                | WellKnownProtocol::Hash
        ),
        // Text: Clone, Eq, Ord, Hash, Default (NOT Copy -- heap-allocated)
        "Text" => matches!(
            proto,
            WellKnownProtocol::Clone
                | WellKnownProtocol::Eq
                | WellKnownProtocol::Ord
                | WellKnownProtocol::Hash
                | WellKnownProtocol::Default
        ),
        // Unit: Copy, Clone, Eq, Ord, Hash, Default
        "Unit" | "()" => matches!(
            proto,
            WellKnownProtocol::Copy
                | WellKnownProtocol::Clone
                | WellKnownProtocol::Eq
                | WellKnownProtocol::Ord
                | WellKnownProtocol::Hash
                | WellKnownProtocol::Default
        ),
        _ => return None, // Not a primitive -- caller should check other sources
    };
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_name() {
        for wkt in [
            WellKnownType::Int,
            WellKnownType::Float,
            WellKnownType::Bool,
            WellKnownType::Text,
            WellKnownType::List,
            WellKnownType::Map,
            WellKnownType::Set,
            WellKnownType::Deque,
            WellKnownType::Maybe,
            WellKnownType::Heap,
            WellKnownType::Channel,
            WellKnownType::Semaphore,
            WellKnownType::BTreeMap,
            WellKnownType::Once,
            WellKnownType::Range,
        ] {
            assert_eq!(WellKnownType::from_name(wkt.as_str()), Some(wkt));
            assert!(wkt.matches(wkt.as_str()));
        }
    }

    #[test]
    fn unknown_name_returns_none() {
        assert_eq!(WellKnownType::from_name("MyCustomType"), None);
    }

    #[test]
    fn classification() {
        assert!(WellKnownType::List.is_collection());
        assert!(WellKnownType::Channel.is_concurrency());
        assert!(WellKnownType::Int.is_primitive());
        assert!(WellKnownType::Maybe.is_wrapper());
        assert!(!WellKnownType::Text.is_collection());
    }

    #[test]
    fn protocol_roundtrip() {
        for wkp in [
            WellKnownProtocol::Copy,
            WellKnownProtocol::Clone,
            WellKnownProtocol::Eq,
            WellKnownProtocol::Ord,
            WellKnownProtocol::Hash,
            WellKnownProtocol::Default,
            WellKnownProtocol::Debug,
            WellKnownProtocol::Display,
            WellKnownProtocol::Iterator,
            WellKnownProtocol::Future,
        ] {
            assert_eq!(WellKnownProtocol::from_name(wkp.as_str()), Some(wkp));
            assert!(wkp.matches(wkp.as_str()));
        }
    }

    #[test]
    fn method_to_protocol_mapping() {
        assert_eq!(method_to_protocol("hash"), Some(WellKnownProtocol::Hash));
        assert_eq!(method_to_protocol("eq"), Some(WellKnownProtocol::Eq));
        assert_eq!(method_to_protocol("clone"), Some(WellKnownProtocol::Clone));
        assert_eq!(
            method_to_protocol("next"),
            Some(WellKnownProtocol::Iterator)
        );
        assert_eq!(method_to_protocol("drop"), Some(WellKnownProtocol::Drop));
        assert_eq!(method_to_protocol("unknown_method"), None);
    }

    #[test]
    fn primitive_protocol_registry() {
        // Int implements Copy, Clone, Eq, Ord, Hash, Default
        assert_eq!(primitive_implements_protocol("Int", "Copy"), Some(true));
        assert_eq!(primitive_implements_protocol("Int", "Clone"), Some(true));
        assert_eq!(primitive_implements_protocol("Int", "Eq"), Some(true));
        assert_eq!(primitive_implements_protocol("Int", "Hash"), Some(true));

        // Float does NOT implement Eq (NaN)
        assert_eq!(primitive_implements_protocol("Float", "Eq"), Some(false));
        assert_eq!(primitive_implements_protocol("Float", "Clone"), Some(true));

        // Text does NOT implement Copy
        assert_eq!(primitive_implements_protocol("Text", "Copy"), Some(false));
        assert_eq!(primitive_implements_protocol("Text", "Clone"), Some(true));

        // Unknown type returns None
        assert_eq!(primitive_implements_protocol("MyType", "Clone"), None);
        // Unknown protocol returns None
        assert_eq!(primitive_implements_protocol("Int", "Serialize"), None);
    }

    #[test]
    fn fat_ref_protocol_check() {
        assert!(WellKnownProtocol::is_fat_ref_protocol("Display"));
        assert!(WellKnownProtocol::is_fat_ref_protocol("Clone"));
        assert!(WellKnownProtocol::is_fat_ref_protocol("Iterator"));
        assert!(!WellKnownProtocol::is_fat_ref_protocol("MyCustomProtocol"));
    }

    /// Pins the full (primitive × protocol) matrix from
    /// `primitive_implements_protocol`. Each row encodes the EXACT set of
    /// protocols a primitive must satisfy. Anyone editing the function above
    /// must update this matrix, and vice versa — silent drift is impossible.
    ///
    /// The truth table is the matching one in
    /// `core-tests/base/protocols/audit.md §2.1`:
    ///
    ///   | Primitive | Copy | Clone | Eq | Ord | Hash | Default |
    ///   |-----------|:----:|:-----:|:--:|:---:|:----:|:-------:|
    ///   | Int       |  ✓   |   ✓   |  ✓ |  ✓  |   ✓  |    ✓    |
    ///   | Float     |  ✓   |   ✓   |  ✗ |  ✗  |   ✗  |    ✓    |  (NaN)
    ///   | Bool      |  ✓   |   ✓   |  ✓ |  ✓  |   ✓  |    ✓    |
    ///   | Char      |  ✓   |   ✓   |  ✓ |  ✓  |   ✓  |    ✗    |
    ///   | Text      |  ✗   |   ✓   |  ✓ |  ✓  |   ✓  |    ✓    |  (heap)
    ///   | Unit      |  ✓   |   ✓   |  ✓ |  ✓  |   ✓  |    ✓    |
    #[test]
    fn primitive_protocol_matrix_pinned() {
        // (type, [(protocol_name, implemented?)])
        let matrix: &[(&str, &[(&str, bool)])] = &[
            (
                "Int",
                &[
                    ("Copy", true),
                    ("Clone", true),
                    ("Eq", true),
                    ("Ord", true),
                    ("Hash", true),
                    ("Default", true),
                ],
            ),
            (
                "Float",
                &[
                    ("Copy", true),
                    ("Clone", true),
                    ("Eq", false),
                    ("Ord", false),
                    ("Hash", false),
                    ("Default", true),
                ],
            ),
            (
                "Bool",
                &[
                    ("Copy", true),
                    ("Clone", true),
                    ("Eq", true),
                    ("Ord", true),
                    ("Hash", true),
                    ("Default", true),
                ],
            ),
            (
                "Char",
                &[
                    ("Copy", true),
                    ("Clone", true),
                    ("Eq", true),
                    ("Ord", true),
                    ("Hash", true),
                    ("Default", false),
                ],
            ),
            (
                "Text",
                &[
                    ("Copy", false),
                    ("Clone", true),
                    ("Eq", true),
                    ("Ord", true),
                    ("Hash", true),
                    ("Default", true),
                ],
            ),
            (
                "Unit",
                &[
                    ("Copy", true),
                    ("Clone", true),
                    ("Eq", true),
                    ("Ord", true),
                    ("Hash", true),
                    ("Default", true),
                ],
            ),
        ];

        for (ty, rows) in matrix {
            for (proto, expected) in *rows {
                let got = primitive_implements_protocol(ty, proto);
                assert_eq!(
                    got,
                    Some(*expected),
                    "matrix drift: primitive_implements_protocol({:?}, {:?}) \
                     returned {:?}, audit.md §2.1 says {:?}",
                    ty,
                    proto,
                    got,
                    expected
                );
            }
        }

        // `()` should resolve identically to `Unit`.
        assert_eq!(primitive_implements_protocol("()", "Copy"), Some(true));
        assert_eq!(primitive_implements_protocol("()", "Default"), Some(true));

        // Unknown primitive → None (caller should check other sources).
        assert_eq!(primitive_implements_protocol("UInt128", "Copy"), None);
        // Unknown protocol → None.
        assert_eq!(primitive_implements_protocol("Int", "NotAProtocol"), None);
    }
}
