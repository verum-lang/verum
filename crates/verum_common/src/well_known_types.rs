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
    Stopwatch,
    PerfCounter,
    DeadlineTimer,

    // Misc
    Never,
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
            Self::Stopwatch => "Stopwatch",
            Self::PerfCounter => "PerfCounter",
            Self::DeadlineTimer => "DeadlineTimer",
            Self::Never => "Never",
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
            "Stopwatch" => Some(Self::Stopwatch),
            "PerfCounter" => Some(Self::PerfCounter),
            "DeadlineTimer" => Some(Self::DeadlineTimer),
            "Never" => Some(Self::Never),
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

    /// Canonical archive-module prefixes that contain this well-known
    /// type's inherent + protocol-impl methods. Used by the archive
    /// lazy-loader (`verum_compiler::archive_ctx_loader`) to expand
    /// `wanted_module_prefixes` when user code mentions a stdlib type
    /// by name (e.g. `Text.new()` should pull in `core.text.text` so
    /// `Text.new` is registered into `ctx.functions` for the user-side
    /// codegen).
    ///
    /// Returns the *direct* archive entry first (e.g.
    /// `"core.text.text"` for files declaring `module core.text.text;`)
    /// followed by the parent-module bundle (e.g. `"core.text"`) since
    /// the precompiler sometimes bundles inherent methods under the
    /// grandparent entry depending on the module hierarchy.
    ///
    /// Returns an empty slice for types without a canonical archive
    /// home (primitives whose methods are interpreter-built-in only:
    /// Int, Float, Bool, Char) — those don't need archive loading.
    ///
    /// **Source-of-truth contract**: each returned string is a
    /// candidate archive-entry name; at least ONE entry per well-known
    /// type must resolve to an actual archive entry. A pin test
    /// (`canonical_archive_modules_match_source` in
    /// `verum_compiler::archive_ctx_loader`) enforces this.  The
    /// alternates list both the source-declared path and the
    /// grandparent-bundled fallback because the precompiler chooses
    /// one or the other depending on hierarchy shape — the loader's
    /// `wanted_module_prefixes` set is happy with whichever resolves.
    pub const fn canonical_archive_modules(self) -> &'static [&'static str] {
        match self {
            // Text family — `core/text/text.vr` declares
            // `module core.text.text;`. The text/ module-group also
            // contains format.vr / builder.vr / regex.vr / numeric.vr;
            // they hang off `Formatter` / `TextBuilder` / `Regex` /
            // `Numeric` (separate well-known names not yet enumerated
            // here — caller should mention them explicitly when used).
            Self::Text | Self::Char => {
                &["core.text.text", "core.text"]
            }
            // Collections — each has its own file under
            // `core/collections/<name>.vr` declaring
            // `module core.collections.<name>;`.
            Self::List => &["core.collections.list", "core.collections"],
            Self::Map => &["core.collections.map", "core.collections"],
            Self::Set => &["core.collections.set", "core.collections"],
            Self::Deque => &["core.collections.deque", "core.collections"],
            Self::BTreeMap => {
                &["core.collections.btree_map", "core.collections"]
            }
            Self::BTreeSet => {
                &["core.collections.btree_set", "core.collections"]
            }
            Self::BinaryHeap => {
                &["core.collections.binary_heap", "core.collections"]
            }
            // Wrappers — `core/base/<name>.vr`.
            Self::Maybe => &["core.base.maybe", "core.base"],
            Self::Result => &["core.base.result", "core.base"],
            Self::Heap => &["core.mem.heap", "core.mem"],
            Self::Shared => &["core.sync.shared", "core.sync"],
            // Concurrency — `core/sync/<name>.vr` (Channel lives in
            // core/async/channel.vr; Mutex/RwLock/Barrier/etc. in
            // core/sync/).
            Self::Channel => &["core.async.channel", "core.async"],
            Self::Mutex => &["core.sync.mutex", "core.sync"],
            Self::RwLock => &["core.sync.rwlock", "core.sync"],
            Self::Barrier => &["core.sync.barrier", "core.sync"],
            Self::WaitGroup => &["core.sync.wait_group", "core.sync"],
            Self::Once => &["core.sync.once", "core.sync"],
            Self::Semaphore => &["core.async.semaphore", "core.async"],
            Self::Task => &["core.async.task", "core.async"],
            Self::Nursery => &["core.async.nursery", "core.async"],
            Self::AtomicInt => &["core.sync.atomic", "core.sync"],
            Self::AtomicBool => &["core.sync.atomic", "core.sync"],
            // Time — `core/time/<name>.vr`.
            Self::Duration => &["core.time.duration", "core.time"],
            Self::Instant => &["core.time.instant", "core.time"],
            Self::Stopwatch => &["core.time.stopwatch", "core.time"],
            Self::PerfCounter => &["core.time.perf_counter", "core.time"],
            Self::DeadlineTimer => &["core.time.deadline_timer", "core.time"],
            // Misc — `core/base/<name>.vr` for Never/Ordering/Range.
            Self::Never => &["core.base"],
            Self::Ordering => &["core.base.ordering", "core.base"],
            Self::Range => &["core.base.range", "core.base"],
            // Primitives — methods are interpreter-built-in only,
            // no archive load needed.
            Self::Int | Self::Float | Self::Bool => &[],
        }
    }
}

/// Conservatively classify a type name as a generic type parameter
/// (e.g. `T`, `E`, `K`, `V`, `R`, `Item`, `Out`).
///
/// Verum's convention follows Rust/Haskell: type parameters are
/// short PascalCase identifiers. The classifier accepts:
///
///   * **1 char**, ASCII uppercase: `T`, `E`, `K`, `V`, `R`, `S`, …
///   * **2 chars**, uppercase + lowercase: `Tk`, `Vk`, `Rs`, …
///
/// 3+-char names like `Item`, `Output`, `Iter` are NOT classified
/// as type params because they collide with concrete type names
/// users define (e.g. `type Item is { … }` is real stdlib code).
/// The grammar disambiguates these via `where T: Trait` clauses; in
/// the absence of an unambiguous syntactic signal, the conservative
/// classifier prevents misclassification of concrete types.
///
/// Used by VBC method-dispatch codegen to detect calls of the form
/// `<generic>.method(...)` and emit the bare method name (letting
/// runtime dispatch route by receiver kind) instead of a
/// `T.method(...)` literal that no method-table entry can resolve.
///
/// Centralised here so the type-inference layer
/// (`verum_types::infer`) and the VBC codegen layer agree on the
/// same classification — drift between them produces silent miscompiles
/// where the inferer treats a name as concrete while codegen treats
/// it as generic (or vice-versa).
pub fn looks_like_type_param(name: &str) -> bool {
    match name.len() {
        1 => name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_uppercase()),
        2 => {
            let mut chars = name.chars();
            match (chars.next(), chars.next()) {
                (Some(first), Some(second)) => {
                    first.is_ascii_uppercase() && second.is_ascii_lowercase()
                }
                _ => false,
            }
        }
        _ => false,
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

    /// Structural shape of a sum-type variant set.
    ///
    /// Identifies canonical Verum stdlib variant patterns by their constructor
    /// names — independent of the nominal type name. Compiler code that needs
    /// to recognize Result-like / Maybe-like values consults this enum rather
    /// than hardcoding variant-name strings.
    ///
    /// **Why structural?** A user-defined `type MyEither<T,E> is Ok(T) | Err(E)`
    /// or an FFI binding emitted as a Result-shaped sum benefits from the same
    /// coercions and projections that apply to the canonical stdlib `Result`.
    /// The shape is a structural concept ("two variants whose names match the
    /// canonical Result layout"), and the canonical names live in this module's
    /// `MAYBE_VARIANT_LAYOUT` / `RESULT_VARIANT_LAYOUT` constants — the same
    /// source of truth used by VBC codegen and the runtime.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum VariantShape {
        /// Two variants whose names are exactly `Ok` and `Err`.
        Result,
        /// Two variants whose names are exactly `Some` and `None`,
        /// or the Haskell-style alias pair `Just`/`Nothing`.
        Maybe,
        /// Any other variant set — including singletons, larger sums,
        /// or two-variant sums whose names don't match a canonical shape.
        Other,
    }

    /// Returns the structural shape of a variant-name set.
    ///
    /// Accepts any iterable of variant constructor names. Order-insensitive;
    /// counts the variants in a single pass and short-circuits on the first
    /// non-canonical name. Allocation-free.
    ///
    /// Canonical-shape membership is derived from `MAYBE_VARIANT_LAYOUT` /
    /// `RESULT_VARIANT_LAYOUT` (plus the `Just`/`Nothing` Haskell aliases
    /// already recognized by [`is_maybe_constructor`]). Editing a layout
    /// constant automatically retunes this classifier — no parallel list.
    pub fn classify_variants<I, S>(names: I) -> VariantShape
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        // Bitfield of which canonical variant names we've seen, plus a
        // `count` to enforce the "exactly N variants" arity check. A
        // single non-canonical name aborts to Other immediately.
        let (mut seen_some, mut seen_none) = (false, false);
        let (mut seen_just, mut seen_nothing) = (false, false);
        let (mut seen_ok, mut seen_err) = (false, false);
        let mut count: u32 = 0;
        for name in names {
            count += 1;
            if count > 2 {
                return VariantShape::Other;
            }
            match name.as_ref() {
                SOME => seen_some = true,
                NONE => seen_none = true,
                JUST => seen_just = true,
                NOTHING => seen_nothing = true,
                OK => seen_ok = true,
                ERR => seen_err = true,
                _ => return VariantShape::Other,
            }
        }
        if count != 2 {
            return VariantShape::Other;
        }
        if (seen_some && seen_none) || (seen_just && seen_nothing) {
            VariantShape::Maybe
        } else if seen_ok && seen_err {
            VariantShape::Result
        } else {
            // Two variants but mixed-shape (e.g., `Some` + `Err`) — not
            // canonical.
            VariantShape::Other
        }
    }

    /// Returns true iff `names` has exactly the Result-shape (`Ok` + `Err`).
    pub fn is_result_shape<I, S>(names: I) -> bool
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        matches!(classify_variants(names), VariantShape::Result)
    }

    /// Returns true iff `names` has exactly the Maybe-shape (`Some`+`None`
    /// or `Just`+`Nothing`).
    pub fn is_maybe_shape<I, S>(names: I) -> bool
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        matches!(classify_variants(names), VariantShape::Maybe)
    }

    /// Extract the `(Ok, Err)` payload pair from a Result-shaped variant
    /// set. Caller provides a getter `(name → Option<T>)` typically backed
    /// by a `HashMap` / `Map`. Returns `None` if either canonical name is
    /// absent.
    ///
    /// The returned pair is in **canonical order** (Ok first, Err second),
    /// matching `RESULT_VARIANT_LAYOUT`.
    pub fn extract_result_pair<T, F>(get: F) -> Option<(T, T)>
    where
        F: Fn(&str) -> Option<T>,
    {
        let ok = get(OK)?;
        let err = get(ERR)?;
        Some((ok, err))
    }

    /// Extract the `Some` payload from a Maybe-shaped variant set.
    /// Tries `Some` first, then the Haskell alias `Just`. Returns `None`
    /// if neither is present.
    pub fn extract_maybe_inner<T, F>(get: F) -> Option<T>
    where
        F: Fn(&str) -> Option<T>,
    {
        get(SOME).or_else(|| get(JUST))
    }
}

/// A canonical entry in a `*_VARIANT_LAYOUT` slice — the
/// single source of truth for one variant of one well-known sum
/// type.
///
/// `name` is the constructor name as it appears in the `.vr`
/// source declaration (e.g. `"None"`, `"Some"`, `"Ok"`,
/// `"Err"`, `"Less"`, …); `tag` is the discriminator value the
/// runtime stores in the variant header; `arity` is the number
/// of payload fields the constructor takes (0 for unit variants
/// like `None` / `Less` / `Greater` / `Null` / `NotPresent`, 1
/// for single-payload variants like `Some(T)` / `Ok(T)` /
/// `Err(E)` / `Continue(C)` / `Break(B)` / `NotUnicode(bytes)`,
/// and N for N-tuple variants — none currently exist in the
/// well-known set but the type admits them).
///
/// **Why arity here?** Pre-fix the layout encoded only
/// `(name, tag)`; arity lived in *each consumer* as a parallel
/// hardcoded set of per-type rules — `register_builtin_variants`
/// in VBC codegen had three ad-hoc loops (a `if name == "Some"`
/// branch for Maybe, a uniform arity-1 hardcode for Result, a
/// uniform arity-0 hardcode for Ordering). Adding a new variant
/// carrier (e.g. `Either<L,R>`) needed both a layout constant
/// AND a new ad-hoc loop with type-specific arity logic.
/// Lifting arity into the layout itself eliminates the
/// parallel-rules drift surface — every consumer derives arity
/// from the same canonical declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VariantLayoutEntry {
    /// Constructor name as written in the `.vr` source.
    pub name: &'static str,
    /// Runtime discriminator value (0-based, declaration order).
    pub tag: u32,
    /// Number of payload fields the constructor takes.
    pub arity: u32,
}

impl VariantLayoutEntry {
    /// Construct a layout entry. `const fn` so it can be used in
    /// `*_VARIANT_LAYOUT` slice literals at module scope.
    #[inline]
    pub const fn new(name: &'static str, tag: u32, arity: u32) -> Self {
        Self { name, tag, arity }
    }
}

/// Canonical layout of the variants of `core::base::maybe::Maybe<T>`.
///
/// Source-of-truth: `core/base/maybe.vr`:
/// ```text
///     public type Maybe<T> is None | Some(T);
/// ```
/// Tags follow declaration order: `None = 0` (arity 0), `Some = 1`
/// (arity 1 — the `T` payload).
///
/// **Drift contract:** any reorder or arity change in the .vr file
/// MUST be reflected here, and the matrix-pinning test in
/// `tests::maybe_variant_layout_pinned` catches the divergence at
/// test time.
pub const MAYBE_VARIANT_LAYOUT: &[VariantLayoutEntry] = &[
    VariantLayoutEntry::new("None", 0, 0),
    VariantLayoutEntry::new("Some", 1, 1),
];

/// Canonical layout of the variants of `core::base::result::Result<T, E>`.
///
/// Source-of-truth: `core/base/result.vr`:
/// ```text
///     public type Result<T, E> is Ok(T) | Err(E);
/// ```
/// Tags follow declaration order: `Ok = 0` (arity 1 — the `T`
/// payload), `Err = 1` (arity 1 — the `E` payload).
pub const RESULT_VARIANT_LAYOUT: &[VariantLayoutEntry] = &[
    VariantLayoutEntry::new("Ok", 0, 1),
    VariantLayoutEntry::new("Err", 1, 1),
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
pub const ORDERING_VARIANT_LAYOUT: &[VariantLayoutEntry] = &[
    VariantLayoutEntry::new("Less", 0, 0),
    VariantLayoutEntry::new("Equal", 1, 0),
    VariantLayoutEntry::new("Greater", 2, 0),
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
pub const DATA_VARIANT_LAYOUT: &[VariantLayoutEntry] = &[
    VariantLayoutEntry::new("Null", 0, 0),
    VariantLayoutEntry::new("Bool", 1, 1),
    VariantLayoutEntry::new("Int", 2, 1),
    VariantLayoutEntry::new("Float", 3, 1),
    VariantLayoutEntry::new("Text", 4, 1),
    VariantLayoutEntry::new("Array", 5, 1),
    VariantLayoutEntry::new("Object", 6, 1),
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
pub const CONTROLFLOW_VARIANT_LAYOUT: &[VariantLayoutEntry] = &[
    VariantLayoutEntry::new("Continue", 0, 1),
    VariantLayoutEntry::new("Break", 1, 1),
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
pub const VARERROR_VARIANT_LAYOUT: &[VariantLayoutEntry] = &[
    VariantLayoutEntry::new("NotPresent", 0, 0),
    VariantLayoutEntry::new("NotUnicode", 1, 1),
];

/// Returns the canonical tag for `VarError::NotPresent`.
///
/// Use this instead of the literal `0` in `wrap_in_variant(state, "VarError", ...)`.
/// Derived from `VARERROR_VARIANT_LAYOUT` via [`tag_of_or_drift`].
pub fn varerror_not_present_tag() -> u32 {
    tag_of_or_drift(VARERROR_VARIANT_LAYOUT, "NotPresent", "VARERROR_VARIANT_LAYOUT")
}

/// Returns the canonical tag for `VarError::NotUnicode`.
///
/// Use this instead of the literal `1` in `wrap_in_variant(state, "VarError", ...)`.
/// Derived from `VARERROR_VARIANT_LAYOUT` via [`tag_of_or_drift`].
pub fn varerror_not_unicode_tag() -> u32 {
    tag_of_or_drift(VARERROR_VARIANT_LAYOUT, "NotUnicode", "VARERROR_VARIANT_LAYOUT")
}

/// Canonical list of marker protocol names in `core::base::protocols`.
///
/// **Definition:** A *marker protocol* is a protocol with zero required or
/// provided methods, zero associated types, and zero associated constants.
/// Its sole purpose is to express a compile-time capability constraint at
/// the type level (e.g., thread-safety, pinning, sizedness).
///
/// **Single source of truth.** Code that registers, queries, or validates
/// marker protocols (in `verum_types/send_sync.rs` and test suites) must
/// iterate this slice rather than duplicating the name list.
///
/// **Drift contract:** any new marker protocol added to `core/base/protocols.vr`
/// MUST be appended here; the `all_marker_protocols_are_registered` test will
/// catch the gap.
pub const MARKER_PROTOCOL_NAMES: &[&str] = &["Sized", "Send", "Sync", "Unpin"];

/// Look up a variant's canonical tag by name in any
/// `*_VARIANT_LAYOUT` slice. Returns `None` when the name is absent —
/// signalling drift between the caller's expectation and the
/// canonical source-of-truth.
///
/// This is the single primitive every per-variant tag accessor in this
/// module composes around. Callers that know the name MUST be present
/// (because they sourced it from the same .vr declaration as the
/// layout) should prefer [`tag_of_or_drift`] which panics with a
/// structured drift message instead of silently producing `None`.
#[inline]
pub fn tag_of(layout: &[VariantLayoutEntry], name: &str) -> Option<u32> {
    layout
        .iter()
        .find_map(|e| (e.name == name).then_some(e.tag))
}

/// Look up a variant's payload arity by name. Mirrors [`tag_of`] for
/// the third axis of the canonical layout — used by the VBC
/// codegen's `register_builtin_variants` so the per-type ad-hoc
/// arity rules collapse into a single uniform layout-driven loop.
#[inline]
pub fn arity_of(layout: &[VariantLayoutEntry], name: &str) -> Option<u32> {
    layout
        .iter()
        .find_map(|e| (e.name == name).then_some(e.arity))
}

/// Like [`tag_of`] but panics with a structured drift message when
/// `name` is absent from `layout`. Use this at sites that source the
/// name from the canonical Verum declaration the layout describes —
/// absence is a programming error, not a recoverable runtime
/// condition.
///
/// `layout_name` is the constant identifier (e.g. `"MAYBE_VARIANT_LAYOUT"`),
/// included in the panic message so the operator can locate the
/// canonical-source-of-truth definition without grep.
#[inline]
pub fn tag_of_or_drift(
    layout: &[VariantLayoutEntry],
    name: &str,
    layout_name: &str,
) -> u32 {
    tag_of(layout, name).unwrap_or_else(|| {
        panic!(
            "{} is missing variant `{}` — drift between caller and the \
             canonical layout. Check the .vr source-of-truth and the \
             layout constant in verum_common/src/well_known_types.rs",
            layout_name, name
        )
    })
}

/// Returns the canonical success variant tag for `Maybe<T>` (the tag for `Some`).
///
/// Derived from `MAYBE_VARIANT_LAYOUT` — same source of truth used by
/// `compile_try`, MakeVariant, and @property generators.
pub fn maybe_success_tag() -> u32 {
    tag_of_or_drift(MAYBE_VARIANT_LAYOUT, "Some", "MAYBE_VARIANT_LAYOUT")
}

/// Returns the canonical `None` variant tag for `Maybe<T>`.
///
/// Mirror of [`maybe_success_tag`]; both derive from the same
/// canonical layout so a future reorder in `core/base/maybe.vr`
/// flows through automatically.
pub fn maybe_none_tag() -> u32 {
    tag_of_or_drift(MAYBE_VARIANT_LAYOUT, "None", "MAYBE_VARIANT_LAYOUT")
}

/// Returns the canonical success variant tag for `Result<T, E>` (the tag for `Ok`).
///
/// Derived from `RESULT_VARIANT_LAYOUT` — same source of truth used by
/// `compile_try`, MakeVariant, and @property generators.
pub fn result_success_tag() -> u32 {
    tag_of_or_drift(RESULT_VARIANT_LAYOUT, "Ok", "RESULT_VARIANT_LAYOUT")
}

/// Returns the canonical `Err` variant tag for `Result<T, E>`.
pub fn result_error_tag() -> u32 {
    tag_of_or_drift(RESULT_VARIANT_LAYOUT, "Err", "RESULT_VARIANT_LAYOUT")
}

/// Look up the canonical Verum tag for a Rust `std::cmp::Ordering` value.
///
/// Translates `std::cmp::Ordering` → variant name → tag from the canonical
/// layout via [`tag_of_or_drift`]. The match below is the std → Verum
/// name table; the layout itself owns the name → tag mapping.
pub fn ordering_tag_for_std(ord: std::cmp::Ordering) -> u32 {
    let name = match ord {
        std::cmp::Ordering::Less => "Less",
        std::cmp::Ordering::Equal => "Equal",
        std::cmp::Ordering::Greater => "Greater",
    };
    tag_of_or_drift(ORDERING_VARIANT_LAYOUT, name, "ORDERING_VARIANT_LAYOUT")
}

#[cfg(test)]
mod ordering_layout_tests {
    use super::*;

    #[test]
    fn layout_pins_canonical_three_variants() {
        // Three variants, in canonical order. If this asserts, either the .vr
        // file changed and the constant must follow, or vice versa — but the
        // load-time validator will already have refused to load.
        // Ordering variants are all unit (arity 0).
        assert_eq!(ORDERING_VARIANT_LAYOUT.len(), 3);
        assert_eq!(ORDERING_VARIANT_LAYOUT[0], VariantLayoutEntry::new("Less", 0, 0));
        assert_eq!(ORDERING_VARIANT_LAYOUT[1], VariantLayoutEntry::new("Equal", 1, 0));
        assert_eq!(ORDERING_VARIANT_LAYOUT[2], VariantLayoutEntry::new("Greater", 2, 0));
    }

    #[test]
    fn ordering_tag_for_std_matches_layout() {
        assert_eq!(ordering_tag_for_std(std::cmp::Ordering::Less), 0);
        assert_eq!(ordering_tag_for_std(std::cmp::Ordering::Equal), 1);
        assert_eq!(ordering_tag_for_std(std::cmp::Ordering::Greater), 2);
    }

    /// Pins the canonical layout of `Maybe<T>`. Mirrors the
    /// Ordering pattern: any change to the variant order or arity
    /// in `core/base/maybe.vr` must be reflected here, and vice
    /// versa. Codegen builtin variant registration consults this
    /// constant for both tag AND arity.
    #[test]
    fn maybe_variant_layout_pinned() {
        assert_eq!(MAYBE_VARIANT_LAYOUT.len(), 2);
        // None is unit (arity 0); Some carries a single payload.
        assert_eq!(MAYBE_VARIANT_LAYOUT[0], VariantLayoutEntry::new("None", 0, 0));
        assert_eq!(MAYBE_VARIANT_LAYOUT[1], VariantLayoutEntry::new("Some", 1, 1));
    }

    /// Pins the canonical layout of `Result<T, E>` — both
    /// constructors carry a single payload (`Ok(T)` / `Err(E)`).
    #[test]
    fn result_variant_layout_pinned() {
        assert_eq!(RESULT_VARIANT_LAYOUT.len(), 2);
        assert_eq!(RESULT_VARIANT_LAYOUT[0], VariantLayoutEntry::new("Ok", 0, 1));
        assert_eq!(RESULT_VARIANT_LAYOUT[1], VariantLayoutEntry::new("Err", 1, 1));
    }

    /// Pins `Data` variant order + arity: Null is unit; the other
    /// six variants each carry a single payload (Bool/Int/Float/
    /// Text/Array(List)/Object(Map)).
    #[test]
    fn data_variant_layout_pinned() {
        assert_eq!(DATA_VARIANT_LAYOUT.len(), 7);
        assert_eq!(DATA_VARIANT_LAYOUT[0], VariantLayoutEntry::new("Null", 0, 0));
        assert_eq!(DATA_VARIANT_LAYOUT[1], VariantLayoutEntry::new("Bool", 1, 1));
        assert_eq!(DATA_VARIANT_LAYOUT[2], VariantLayoutEntry::new("Int", 2, 1));
        assert_eq!(DATA_VARIANT_LAYOUT[3], VariantLayoutEntry::new("Float", 3, 1));
        assert_eq!(DATA_VARIANT_LAYOUT[4], VariantLayoutEntry::new("Text", 4, 1));
        assert_eq!(DATA_VARIANT_LAYOUT[5], VariantLayoutEntry::new("Array", 5, 1));
        assert_eq!(DATA_VARIANT_LAYOUT[6], VariantLayoutEntry::new("Object", 6, 1));
    }

    /// Pins `ControlFlow<B,C>` variant order + arity: both
    /// constructors carry a single payload (`Continue(C)` /
    /// `Break(B)`).
    #[test]
    fn controlflow_variant_layout_pinned() {
        assert_eq!(CONTROLFLOW_VARIANT_LAYOUT.len(), 2);
        assert_eq!(CONTROLFLOW_VARIANT_LAYOUT[0], VariantLayoutEntry::new("Continue", 0, 1));
        assert_eq!(CONTROLFLOW_VARIANT_LAYOUT[1], VariantLayoutEntry::new("Break", 1, 1));
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
            .find_map(|e| (e.name == "Some").then_some(e.tag))
            .unwrap();
        assert_eq!(maybe_success_tag(), maybe_some_tag);

        let result_ok_tag = RESULT_VARIANT_LAYOUT
            .iter()
            .find_map(|e| (e.name == "Ok").then_some(e.tag))
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
            .find_map(|e| (e.name == "Continue").then_some(e.tag))
            .expect("CONTROLFLOW_VARIANT_LAYOUT must contain 'Continue'");
        let break_tag = CONTROLFLOW_VARIANT_LAYOUT
            .iter()
            .find_map(|e| (e.name == "Break").then_some(e.tag))
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
            .find_map(|e| (e.name == "None").then_some(e.tag))
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
            .find_map(|e| (e.name == "Err").then_some(e.tag))
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
        let layouts: &[(&[VariantLayoutEntry], &str, usize)] = &[
            (MAYBE_VARIANT_LAYOUT, "MAYBE", 2),
            (RESULT_VARIANT_LAYOUT, "RESULT", 2),
            (CONTROLFLOW_VARIANT_LAYOUT, "CONTROLFLOW", 2),
            (VARERROR_VARIANT_LAYOUT, "VARERROR", 2),
            (DATA_VARIANT_LAYOUT, "DATA", 7),
            (ORDERING_VARIANT_LAYOUT, "ORDERING", 3),
        ];

        for &(layout, name, expected_len) in layouts {
            assert_eq!(layout.len(), expected_len, "{} layout must have {} variants", name, expected_len);
            let tags: std::collections::HashSet<u32> = layout.iter().map(|e| e.tag).collect();
            assert_eq!(tags.len(), expected_len, "{} layout must have unique tags", name);
            let names: std::collections::HashSet<&str> = layout.iter().map(|e| e.name).collect();
            assert_eq!(names.len(), expected_len, "{} layout must have unique variant names", name);
        }
    }

    /// Pins `VarError` variant order + arity: NotPresent=0 (unit),
    /// NotUnicode=1 (carries a `List<Byte>` payload). The runtime
    /// env intercept (`env_runtime.rs`) must use these tags.
    #[test]
    fn varerror_variant_layout_pinned() {
        assert_eq!(VARERROR_VARIANT_LAYOUT.len(), 2);
        assert_eq!(VARERROR_VARIANT_LAYOUT[0], VariantLayoutEntry::new("NotPresent", 0, 0));
        assert_eq!(VARERROR_VARIANT_LAYOUT[1], VariantLayoutEntry::new("NotUnicode", 1, 1));
    }

    /// Cross-cutting pin: every variant in every well-known layout
    /// has the arity stated by its `.vr` source declaration. Adding
    /// a new layout constant or new variant entry **MUST** include
    /// the right `arity` value — the canonical-layout type
    /// statically refuses to compile without it, and this test
    /// verifies the value matches the .vr-declared shape.
    #[test]
    fn arity_matches_canonical_declarations() {
        // (layout, layout-name, expected (variant-name, expected-arity) pairs)
        let cases: &[(&[VariantLayoutEntry], &str, &[(&str, u32)])] = &[
            (MAYBE_VARIANT_LAYOUT, "MAYBE", &[("None", 0), ("Some", 1)]),
            (RESULT_VARIANT_LAYOUT, "RESULT", &[("Ok", 1), ("Err", 1)]),
            (ORDERING_VARIANT_LAYOUT, "ORDERING", &[("Less", 0), ("Equal", 0), ("Greater", 0)]),
            (
                DATA_VARIANT_LAYOUT,
                "DATA",
                &[
                    ("Null", 0),
                    ("Bool", 1),
                    ("Int", 1),
                    ("Float", 1),
                    ("Text", 1),
                    ("Array", 1),
                    ("Object", 1),
                ],
            ),
            (CONTROLFLOW_VARIANT_LAYOUT, "CONTROLFLOW", &[("Continue", 1), ("Break", 1)]),
            (VARERROR_VARIANT_LAYOUT, "VARERROR", &[("NotPresent", 0), ("NotUnicode", 1)]),
        ];
        for (layout, layout_name, expected_pairs) in cases {
            for (variant_name, expected_arity) in *expected_pairs {
                assert_eq!(
                    arity_of(layout, variant_name),
                    Some(*expected_arity),
                    "{} layout: arity_of({:?}) should be {}",
                    layout_name,
                    variant_name,
                    expected_arity,
                );
            }
        }
    }

    #[test]
    fn varerror_tag_helpers_consistent_with_layout() {
        assert_eq!(varerror_not_present_tag(), 0);
        assert_eq!(varerror_not_unicode_tag(), 1);
        assert_ne!(varerror_not_present_tag(), varerror_not_unicode_tag());
    }

    // =========================================================================
    // tag_of / tag_of_or_drift — canonical layout-name → tag primitive
    //
    // These pin the contract that every per-variant tag accessor in this
    // module routes through the same primitive: a future caller adding a
    // new variant lookup gets the drift-protection panic for free, and a
    // future reorder of the canonical layout flows through to every
    // accessor automatically.
    // =========================================================================

    /// `tag_of` returns the right tag for every name in a layout.
    #[test]
    fn tag_of_resolves_every_canonical_layout_entry() {
        let layouts: &[(&[VariantLayoutEntry], &str)] = &[
            (MAYBE_VARIANT_LAYOUT, "MAYBE"),
            (RESULT_VARIANT_LAYOUT, "RESULT"),
            (ORDERING_VARIANT_LAYOUT, "ORDERING"),
            (DATA_VARIANT_LAYOUT, "DATA"),
            (CONTROLFLOW_VARIANT_LAYOUT, "CONTROLFLOW"),
            (VARERROR_VARIANT_LAYOUT, "VARERROR"),
        ];
        for (layout, layout_name) in layouts {
            for entry in *layout {
                assert_eq!(
                    tag_of(layout, entry.name),
                    Some(entry.tag),
                    "{} layout: tag_of({:?}) should return Some({})",
                    layout_name,
                    entry.name,
                    entry.tag,
                );
            }
        }
    }

    /// `tag_of` returns `None` for names that don't appear in the layout.
    #[test]
    fn tag_of_rejects_unknown_names() {
        assert_eq!(tag_of(MAYBE_VARIANT_LAYOUT, "Just"), None);
        assert_eq!(tag_of(RESULT_VARIANT_LAYOUT, ""), None);
        assert_eq!(tag_of(ORDERING_VARIANT_LAYOUT, "less"), None); // case-sensitive
    }

    /// Every per-variant accessor in this module agrees with
    /// `tag_of_or_drift` against the same layout — this is the
    /// drift-protection invariant that lets us swap any accessor for
    /// the primitive without breaking callers.
    #[test]
    fn per_variant_accessors_agree_with_primitive() {
        assert_eq!(
            maybe_success_tag(),
            tag_of_or_drift(MAYBE_VARIANT_LAYOUT, "Some", "MAYBE_VARIANT_LAYOUT"),
        );
        assert_eq!(
            maybe_none_tag(),
            tag_of_or_drift(MAYBE_VARIANT_LAYOUT, "None", "MAYBE_VARIANT_LAYOUT"),
        );
        assert_eq!(
            result_success_tag(),
            tag_of_or_drift(RESULT_VARIANT_LAYOUT, "Ok", "RESULT_VARIANT_LAYOUT"),
        );
        assert_eq!(
            result_error_tag(),
            tag_of_or_drift(RESULT_VARIANT_LAYOUT, "Err", "RESULT_VARIANT_LAYOUT"),
        );
        assert_eq!(
            varerror_not_present_tag(),
            tag_of_or_drift(VARERROR_VARIANT_LAYOUT, "NotPresent", "VARERROR_VARIANT_LAYOUT"),
        );
        assert_eq!(
            varerror_not_unicode_tag(),
            tag_of_or_drift(VARERROR_VARIANT_LAYOUT, "NotUnicode", "VARERROR_VARIANT_LAYOUT"),
        );
        assert_eq!(
            ordering_tag_for_std(std::cmp::Ordering::Less),
            tag_of_or_drift(ORDERING_VARIANT_LAYOUT, "Less", "ORDERING_VARIANT_LAYOUT"),
        );
    }

    /// `maybe_none_tag()` returns the canonical None tag (0). Pairs with
    /// `maybe_success_tag()` for the Some/None pair-asymmetry contract:
    /// before this addition only `Some` had a named accessor, leaving
    /// `None` callers using `0` literals.
    #[test]
    fn maybe_none_tag_is_zero() {
        assert_eq!(maybe_none_tag(), 0, "None is tag 0 per MAYBE_VARIANT_LAYOUT");
        assert_ne!(maybe_none_tag(), maybe_success_tag());
    }

    /// `result_error_tag()` returns the canonical Err tag (1). Pairs
    /// with `result_success_tag()` for symmetry — same rationale as
    /// `maybe_none_tag` above.
    #[test]
    fn result_error_tag_is_one() {
        assert_eq!(result_error_tag(), 1, "Err is tag 1 per RESULT_VARIANT_LAYOUT");
        assert_ne!(result_error_tag(), result_success_tag());
    }

    /// `tag_of_or_drift` panics with a structured message that names
    /// the layout when the variant is absent — the operator can locate
    /// the canonical-source-of-truth without grep.
    #[test]
    #[should_panic(expected = "MAYBE_VARIANT_LAYOUT is missing variant `Bogus`")]
    fn tag_of_or_drift_panics_with_layout_name() {
        let _ = tag_of_or_drift(MAYBE_VARIANT_LAYOUT, "Bogus", "MAYBE_VARIANT_LAYOUT");
    }

    // =========================================================================
    // VariantShape — structural variant-set classifier
    //
    // The shape classifier replaces hardcoded `variants.contains_key("Ok")` /
    // `... ("Err")` / `... ("Some")` / `... ("None")` checks scattered across
    // unify.rs / ty.rs. Tests below pin both the recognition contract and the
    // canonical-order guarantees relied on by extract_result_pair /
    // extract_maybe_inner.
    // =========================================================================

    use super::variant_tags::{
        VariantShape, classify_variants, extract_maybe_inner, extract_result_pair,
        is_maybe_shape, is_result_shape,
    };

    #[test]
    fn classify_recognizes_result_shape() {
        assert_eq!(
            classify_variants(["Ok", "Err"].iter().copied()),
            VariantShape::Result
        );
        // Order-insensitive
        assert_eq!(
            classify_variants(["Err", "Ok"].iter().copied()),
            VariantShape::Result
        );
    }

    #[test]
    fn classify_recognizes_maybe_shape_canonical() {
        assert_eq!(
            classify_variants(["Some", "None"].iter().copied()),
            VariantShape::Maybe
        );
        assert_eq!(
            classify_variants(["None", "Some"].iter().copied()),
            VariantShape::Maybe
        );
    }

    #[test]
    fn classify_recognizes_maybe_haskell_alias() {
        // Just / Nothing are recognized as the same shape as Some / None
        // because is_maybe_constructor accepts them as aliases.
        assert_eq!(
            classify_variants(["Just", "Nothing"].iter().copied()),
            VariantShape::Maybe
        );
    }

    #[test]
    fn classify_rejects_mixed_shapes() {
        // Two valid canonical names but cross-shape — not a canonical pair.
        assert_eq!(
            classify_variants(["Some", "Err"].iter().copied()),
            VariantShape::Other
        );
        assert_eq!(
            classify_variants(["Ok", "None"].iter().copied()),
            VariantShape::Other
        );
        // Same-family but wrong pair (Some/Just instead of Some/None).
        assert_eq!(
            classify_variants(["Some", "Just"].iter().copied()),
            VariantShape::Other
        );
    }

    #[test]
    fn classify_rejects_non_canonical_names() {
        // User-defined types with arbitrary variant names are not recognized.
        assert_eq!(
            classify_variants(["Cons", "Nil"].iter().copied()),
            VariantShape::Other
        );
        assert_eq!(
            classify_variants(["Left", "Right"].iter().copied()),
            VariantShape::Other
        );
    }

    #[test]
    fn classify_rejects_arity_mismatch() {
        // Singleton — wrong arity.
        assert_eq!(
            classify_variants(["Ok"].iter().copied()),
            VariantShape::Other
        );
        // Three variants — wrong arity even with two canonical names.
        assert_eq!(
            classify_variants(["Ok", "Err", "Pending"].iter().copied()),
            VariantShape::Other
        );
        // Empty — wrong arity.
        let empty: [&str; 0] = [];
        assert_eq!(classify_variants(empty.iter().copied()), VariantShape::Other);
    }

    #[test]
    fn is_result_shape_predicate() {
        assert!(is_result_shape(["Ok", "Err"].iter().copied()));
        assert!(!is_result_shape(["Some", "None"].iter().copied()));
        assert!(!is_result_shape(["Cons", "Nil"].iter().copied()));
    }

    #[test]
    fn is_maybe_shape_predicate() {
        assert!(is_maybe_shape(["Some", "None"].iter().copied()));
        assert!(is_maybe_shape(["Just", "Nothing"].iter().copied()));
        assert!(!is_maybe_shape(["Ok", "Err"].iter().copied()));
        assert!(!is_maybe_shape(["Cons", "Nil"].iter().copied()));
    }

    #[test]
    fn extract_result_pair_canonical_order() {
        // The pair always returns (Ok, Err) regardless of the underlying
        // map iteration order — matching RESULT_VARIANT_LAYOUT.
        let map = std::collections::HashMap::from([
            ("Err", "ErrType"),
            ("Ok", "OkType"),
        ]);
        let pair = extract_result_pair(|name| map.get(name).copied());
        assert_eq!(pair, Some(("OkType", "ErrType")));
    }

    #[test]
    fn extract_result_pair_returns_none_on_missing() {
        // Missing one half of the pair → None (not a partial extraction).
        let map = std::collections::HashMap::from([("Ok", 1)]);
        let pair = extract_result_pair(|name| map.get(name).copied());
        assert_eq!(pair, None);
    }

    #[test]
    fn extract_maybe_inner_prefers_some() {
        // When both Some and Just exist (degenerate but possible), Some wins.
        let map = std::collections::HashMap::from([
            ("Some", "SomeT"),
            ("Just", "JustT"),
        ]);
        let inner = extract_maybe_inner(|name| map.get(name).copied());
        assert_eq!(inner, Some("SomeT"));
    }

    #[test]
    fn extract_maybe_inner_falls_back_to_just() {
        // Only Just present (Haskell-style declarations) — extracted.
        let map = std::collections::HashMap::from([("Just", "JustT")]);
        let inner = extract_maybe_inner(|name| map.get(name).copied());
        assert_eq!(inner, Some("JustT"));
    }

    #[test]
    fn extract_maybe_inner_returns_none_when_absent() {
        let map = std::collections::HashMap::<&str, i32>::new();
        let inner = extract_maybe_inner(|name| map.get(name).copied());
        assert_eq!(inner, None);
    }

    /// Layout / classifier consistency: a variant set populated from
    /// MAYBE_VARIANT_LAYOUT / RESULT_VARIANT_LAYOUT must classify as
    /// Maybe / Result respectively. This is the load-bearing contract
    /// that lets compiler code recognize canonical stdlib types
    /// without naming them.
    #[test]
    fn classifier_consistent_with_canonical_layouts() {
        let maybe_names: Vec<&'static str> =
            MAYBE_VARIANT_LAYOUT.iter().map(|e| e.name).collect();
        assert!(is_maybe_shape(maybe_names.iter().copied()));

        let result_names: Vec<&'static str> =
            RESULT_VARIANT_LAYOUT.iter().map(|e| e.name).collect();
        assert!(is_result_shape(result_names.iter().copied()));
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
    ///
    /// Recognises all three naming conventions in lock-step with
    /// `verum_common::layout::primitive_size_by_name`:
    /// canonical Verum (`Int`, `Int8`..`Int128`, `IntSize`),
    /// legacy uppercase-short (`I8`..`I128`, `Isize`),
    /// and Rust-style lowercase (`i8`..`i128`, `isize`).
    pub fn is_signed_integer_type(name: &str) -> bool {
        matches!(
            name,
            // Canonical Verum
            "Int" | "Int8" | "Int16" | "Int32" | "Int64" | "Int128" | "IntSize"
            // Legacy uppercase-short Verum aliases
            | "I8" | "I16" | "I32" | "I64" | "I128" | "Isize"
            // Rust-style lowercase aliases
            | "i8" | "i16" | "i32" | "i64" | "i128" | "isize"
        )
    }

    /// Returns true if `name` is an unsigned integer type.
    ///
    /// Recognises canonical Verum (`UInt8`..`UInt128`, `USize`, `UIntSize`,
    /// `Byte`), legacy uppercase-short (`U8`..`U128`, `Usize`), and
    /// Rust-style lowercase (`u8`..`u128`, `usize`).
    pub fn is_unsigned_integer_type(name: &str) -> bool {
        matches!(
            name,
            // Canonical Verum
            "UInt8" | "UInt16" | "UInt32" | "UInt64" | "UInt128"
            | "UIntSize" | "USize" | "Byte"
            // Legacy uppercase-short Verum aliases
            | "U8" | "U16" | "U32" | "U64" | "U128" | "Usize"
            // Rust-style lowercase aliases
            | "u8" | "u16" | "u32" | "u64" | "u128" | "usize"
        )
    }

    /// Returns true if `name` is any float type variant.
    ///
    /// Recognises canonical Verum (`Float`, `Float32`, `Float64`),
    /// legacy uppercase-short (`F32`, `F64`), and Rust-style lowercase
    /// (`f32`, `f64`).
    pub fn is_float_type(name: &str) -> bool {
        matches!(
            name,
            "Float" | "Float32" | "Float64"
            | "F32" | "F64"
            | "f32" | "f64"
        )
    }

    /// Returns true if `name` is a primitive value type (no heap allocation
    /// needed). Includes scalar numerics in all three naming conventions
    /// plus `Bool` / `Char` / `Unit` / `()` / `Never`. Excludes `Text`
    /// (heap-backed value type).
    pub fn is_primitive_value_type(name: &str) -> bool {
        // The numeric set is the union of integer + float predicates;
        // delegating keeps the alias coverage in lock-step.
        is_numeric_type(name)
            || matches!(
                name,
                "Bool" | "bool"
                | "Char" | "char"
                | "Unit" | "()"
                | "Never"
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
    ///
    /// Recognises every alias accepted by [`is_signed_integer_type`] /
    /// [`is_unsigned_integer_type`] / [`is_float_type`] — drift between
    /// these tables and the canonical
    /// `verum_common::layout::primitive_size_by_name` is pinned by the
    /// alias-consistency tests in this module.
    pub fn numeric_bit_width(name: &str) -> Option<u32> {
        match name {
            // 8-bit
            "Int8" | "UInt8" | "Byte" | "I8" | "U8" | "i8" | "u8" | "Bool" | "bool" => Some(8),
            // 16-bit
            "Int16" | "UInt16" | "I16" | "U16" | "i16" | "u16" => Some(16),
            // 32-bit
            "Int32" | "UInt32" | "Float32" | "I32" | "U32" | "F32"
            | "i32" | "u32" | "f32" => Some(32),
            // 64-bit (incl. pointer-width aliases)
            "Int" | "Int64" | "UInt64" | "Float" | "Float64"
            | "I64" | "U64" | "F64"
            | "i64" | "u64" | "f64"
            | "IntSize" | "USize" | "UIntSize" | "Isize" | "Usize" | "isize" | "usize" => Some(64),
            // 128-bit
            "Int128" | "UInt128" | "I128" | "U128" | "i128" | "u128" => Some(128),
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
    fn looks_like_type_param_matches_convention() {
        // 1-char uppercase ASCII — canonical Rust/Haskell convention.
        for name in ["T", "U", "V", "K", "E", "R", "S", "A", "B"] {
            assert!(
                looks_like_type_param(name),
                "1-char uppercase '{}' should look like type param",
                name
            );
        }
        // 2-char Pascal-style: uppercase + lowercase.
        for name in ["Tk", "Vk", "Rs", "Ts", "Ok"] {
            assert!(
                looks_like_type_param(name),
                "2-char Pascal '{}' should look like type param",
                name
            );
        }
        // Concrete stdlib type names — must NOT classify.
        for name in [
            "Int", "Bool", "Text", "List", "Map", "Maybe", "Result", "Item",
            "Cell", "Iter", "Self", "TT",
        ] {
            assert!(
                !looks_like_type_param(name),
                "concrete name '{}' must not be classified as type param",
                name
            );
        }
        // Lowercase or 1-letter lowercase — never type params.
        for name in ["t", "x", "abc", ""] {
            assert!(
                !looks_like_type_param(name),
                "non-uppercase '{}' must not classify",
                name
            );
        }
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

    // =========================================================================
    // Naming-convention drift protection
    //
    // The predicates `is_signed_integer_type` / `is_unsigned_integer_type` /
    // `is_float_type` / `is_primitive_value_type` / `numeric_bit_width` and
    // the `verum_common::layout::primitive_size_by_name` table must agree on
    // their alias set: any name resolved by `primitive_size_by_name` to a
    // numeric width MUST be classified by the matching predicate, and vice
    // versa. Drift here would silently make two equivalent source spellings
    // dispatch to different codegen paths or stack-budget rules.
    // =========================================================================

    /// The full canonical / legacy-uppercase / Rust-lowercase alias matrix
    /// for numeric scalars. Each row is `(canonical, [aliases...])`.
    /// Tests below iterate this matrix to pin invariants without relying
    /// on the predicates' internal pattern shape.
    const NUMERIC_ALIAS_MATRIX: &[(&str, u32, bool, bool, &[&str])] = &[
        // (canonical, bit_width, is_signed, is_float, [other accepted names])
        ("Int",     64,  true,  false, &["i64"]),
        ("Int8",    8,   true,  false, &["I8", "i8"]),
        ("Int16",   16,  true,  false, &["I16", "i16"]),
        ("Int32",   32,  true,  false, &["I32", "i32"]),
        ("Int64",   64,  true,  false, &["I64", "i64"]),
        ("Int128",  128, true,  false, &["I128", "i128"]),
        ("IntSize", 64,  true,  false, &["Isize", "isize"]),
        ("UInt8",   8,   false, false, &["U8", "u8", "Byte"]),
        ("UInt16",  16,  false, false, &["U16", "u16"]),
        ("UInt32",  32,  false, false, &["U32", "u32"]),
        ("UInt64",  64,  false, false, &["U64", "u64"]),
        ("UInt128", 128, false, false, &["U128", "u128"]),
        ("USize",   64,  false, false, &["UIntSize", "Usize", "usize"]),
        ("Float",   64,  false, true,  &["f64"]),
        ("Float32", 32,  false, true,  &["F32", "f32"]),
        ("Float64", 64,  false, true,  &["F64", "f64"]),
    ];

    /// Every alias in the matrix is recognized by `is_primitive_value_type`,
    /// `is_numeric_type`, and the appropriate signed/unsigned/float predicate.
    #[test]
    fn alias_matrix_classification_pinned() {
        for (canon, _bits, is_signed, is_float, aliases) in NUMERIC_ALIAS_MATRIX {
            let names = std::iter::once(*canon).chain(aliases.iter().copied());
            for n in names {
                assert!(
                    type_names::is_primitive_value_type(n),
                    "is_primitive_value_type({:?}) must be true (canonical {:?})",
                    n, canon,
                );
                assert!(
                    type_names::is_numeric_type(n),
                    "is_numeric_type({:?}) must be true (canonical {:?})",
                    n, canon,
                );
                if *is_float {
                    assert!(
                        type_names::is_float_type(n),
                        "is_float_type({:?}) must be true (canonical {:?})",
                        n, canon,
                    );
                    assert!(
                        !type_names::is_integer_type(n),
                        "is_integer_type({:?}) must be false for float canonical {:?}",
                        n, canon,
                    );
                } else {
                    assert!(
                        type_names::is_integer_type(n),
                        "is_integer_type({:?}) must be true (canonical {:?})",
                        n, canon,
                    );
                    if *is_signed {
                        assert!(
                            type_names::is_signed_integer_type(n),
                            "is_signed_integer_type({:?}) must be true",
                            n,
                        );
                        assert!(
                            !type_names::is_unsigned_integer_type(n),
                            "is_unsigned_integer_type({:?}) must be false",
                            n,
                        );
                    } else {
                        assert!(
                            type_names::is_unsigned_integer_type(n),
                            "is_unsigned_integer_type({:?}) must be true",
                            n,
                        );
                        assert!(
                            !type_names::is_signed_integer_type(n),
                            "is_signed_integer_type({:?}) must be false",
                            n,
                        );
                    }
                }
            }
        }
    }

    /// `numeric_bit_width` agrees with the matrix for every alias.
    #[test]
    fn alias_matrix_bit_width_pinned() {
        for (canon, bits, _is_signed, _is_float, aliases) in NUMERIC_ALIAS_MATRIX {
            let names = std::iter::once(*canon).chain(aliases.iter().copied());
            for n in names {
                assert_eq!(
                    type_names::numeric_bit_width(n),
                    Some(*bits),
                    "numeric_bit_width({:?}) ≠ {} (canonical {:?})",
                    n, bits, canon,
                );
            }
        }
    }

    /// `layout::primitive_size_by_name` agrees on byte width
    /// (8 × bit_width per primitive). Single drift contract: the
    /// layout module is the size oracle, the type_names module is the
    /// classification oracle — they MUST agree on the alias set and
    /// on the byte/bit conversion.
    #[test]
    fn alias_matrix_layout_consistency_pinned() {
        use crate::layout::primitive_size_by_name;
        for (canon, bits, _is_signed, _is_float, aliases) in NUMERIC_ALIAS_MATRIX {
            let names = std::iter::once(*canon).chain(aliases.iter().copied());
            let expected_bytes = (*bits / 8) as u64;
            for n in names {
                assert_eq!(
                    primitive_size_by_name(n),
                    Some(expected_bytes),
                    "layout::primitive_size_by_name({:?}) ≠ {} (canonical {:?})",
                    n, expected_bytes, canon,
                );
            }
        }
    }

    /// Bool / Char / Unit / () / Never are primitive value types but not
    /// numeric — pin the boundary explicitly (a regression here would
    /// silently make `is_numeric_type("Bool")` true and break
    /// arithmetic-only optimisation passes).
    #[test]
    fn non_numeric_primitives_pinned() {
        for n in ["Bool", "bool", "Char", "char", "Unit", "()", "Never"] {
            assert!(
                type_names::is_primitive_value_type(n),
                "{:?} must classify as primitive value",
                n,
            );
            assert!(
                !type_names::is_numeric_type(n),
                "{:?} must NOT classify as numeric",
                n,
            );
            assert!(!type_names::is_integer_type(n));
            assert!(!type_names::is_float_type(n));
        }
    }

    /// Compound / unknown names: every predicate returns false.
    #[test]
    fn compound_and_unknown_names_rejected() {
        for n in ["List", "Map", "Set", "Maybe", "Result", "MyType", "T", "Item"] {
            assert!(!type_names::is_primitive_value_type(n));
            assert!(!type_names::is_numeric_type(n));
            assert!(!type_names::is_integer_type(n));
            assert!(!type_names::is_float_type(n));
            assert_eq!(type_names::numeric_bit_width(n), None);
        }
        // Text is value-typed but heap-backed — explicitly excluded
        // from the primitive-value classification.
        assert!(!type_names::is_primitive_value_type("Text"));
    }
}
