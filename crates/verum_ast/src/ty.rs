//! Type nodes in the AST.
//!
//! This module defines all type representations in Verum, including:
//! - Primitive types (Int, Float, Bool, Text)
//! - Compound types (tuples, arrays, functions)
//! - Refinement types (the core innovation of Verum)
//! - Generic types with const parameters
//! - Three-tier reference model (&T, &checked T, &unsafe T)

use crate::context::ContextList;
use crate::decl::RecordField;
use crate::ffi::CallingConvention;
use crate::span::{Span, Spanned};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use verum_common::{Heap, List, Maybe, Text};

/// A type in the Verum type system.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Type {
    pub kind: TypeKind,
    pub span: Span,
}

impl Type {
    pub fn new(kind: TypeKind, span: Span) -> Self {
        Self { kind, span }
    }

    pub fn unit(span: Span) -> Self {
        Self::new(TypeKind::Unit, span)
    }

    pub fn bool(span: Span) -> Self {
        Self::new(TypeKind::Bool, span)
    }

    pub fn int(span: Span) -> Self {
        Self::new(TypeKind::Int, span)
    }

    pub fn float(span: Span) -> Self {
        Self::new(TypeKind::Float, span)
    }

    pub fn text(span: Span) -> Self {
        Self::new(TypeKind::Text, span)
    }

    pub fn inferred(span: Span) -> Self {
        Self::new(TypeKind::Inferred, span)
    }

    /// Creates a never type (`!`).
    ///
    /// The never type represents the type of expressions that never return,
    /// such as `panic()`, `exit()`, `abort()`, or infinite loops.
    ///
    /// The never type is the bottom type - it is a subtype of all other types.
    pub fn never(span: Span) -> Self {
        Self::new(TypeKind::Never, span)
    }

    /// Creates an unknown type (`unknown`).
    ///
    /// The unknown type is the top type - any value can be assigned to it,
    /// but nothing can be done with it without explicit type narrowing
    /// (via pattern matching or type guards like `x is T`).
    ///
    /// The unknown type is the top type: any value can be assigned to it, but
    /// nothing can be done without explicit type narrowing via `x is T` guards
    /// or pattern matching. Subtyping: T <: unknown for all T.
    pub fn unknown(span: Span) -> Self {
        Self::new(TypeKind::Unknown, span)
    }
}

impl Spanned for Type {
    fn span(&self) -> Span {
        self.span
    }
}

/// The kind of type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TypeKind {
    /// Unit type: ()
    Unit,

    /// Never type: !
    /// The type of expressions that never return (diverging expressions).
    /// Examples: panic(), exit(), infinite loops, return/break/continue.
    ///
    /// The never type is the bottom type - it is a subtype of all other types.
    /// This means a diverging expression can be used wherever any type is expected.
    ///
    /// ```verum
    /// fn abort() -> ! {
    ///     panic("Fatal error");
    /// }
    ///
    /// fn get_value(opt: Maybe<Int>) -> Int {
    ///     opt.unwrap_or_else(|| panic("No value"))  // panic returns !
    /// }
    /// ```
    Never,

    /// Unknown type: unknown
    /// The top type - any value can be assigned to it, but nothing can be done
    /// with it without explicit type narrowing (via pattern matching or type guards).
    ///
    /// Subtyping rules:
    /// - T <: unknown (any type is a subtype of unknown)
    /// - unknown <: T only if T == unknown
    ///
    /// # Examples
    /// ```verum
    /// // Any value can be assigned to unknown
    /// let x: unknown = 42;
    /// let y: unknown = "hello";
    /// let z: unknown = User { name: "Alice" };
    ///
    /// // But nothing can be done without narrowing
    /// // x + 1;       // ERROR: unknown doesn't support +
    /// // x.method();  // ERROR: unknown has no methods
    ///
    /// // Type narrowing via pattern matching
    /// match x {
    ///     x is Int => process_int(x),   // x: Int here
    ///     x is Text => process_text(x), // x: Text here
    ///     _ => handle_other(),
    /// }
    ///
    /// // Type narrowing via guard
    /// if x is User {
    ///     print(x.name);  // x: User here (flow-sensitive)
    /// }
    /// ```
    Unknown,

    /// Primitive types
    Bool,
    Int,
    Float,
    Char,
    Text,

    /// Named type (identifier or path)
    Path(Path),

    /// Path type: `Path<A>(a, b)` — propositional equality path from
    /// `a` to `b` in carrier type `A`. Lowered to `Type::Eq` in the
    /// type checker. Grammar: `path_type_expr` (verum.ebnf line 1049).
    PathType {
        carrier: Heap<Type>,
        lhs: Heap<Expr>,
        rhs: Heap<Expr>,
    },

    /// General dependent type application: `T<A1, A2>(v1, v2, v3)`.
    ///
    /// Covers the full scheme of type constructors indexed by values,
    /// e.g. `Fiber<A, B>(f, b)`, `IsContrMap<A, B>(f)`,
    /// `Glue<A>(phi, T, e)` used throughout `core/math/hott.vr`,
    /// `cubical.vr`, `infinity_topos.vr`, `kan_extension.vr`. The
    /// two-argument `Path<A>(a, b)` keeps its own sugared variant
    /// (`PathType`) for backward compatibility with the existing
    /// elaboration path; everything else goes through here.
    ///
    /// `carrier` is the type head (already includes the generic
    /// `<…>` args as a `TypeKind::Generic`); `value_args` is the
    /// positional list of index expressions.
    DependentApp {
        carrier: Heap<Type>,
        value_args: List<Expr>,
    },

    /// Tuple type: (T1, T2, ...)
    Tuple(List<Type>),

    /// Array type: [T; N]
    Array {
        element: Heap<Type>,
        size: Maybe<Heap<Expr>>,
    },

    /// Slice type: [T]
    Slice(Heap<Type>),

    /// Function type: fn(A, B) -> C or extern "C" fn(A, B) -> C
    ///
    /// Supports optional calling convention for FFI function pointer types.
    /// When `calling_convention` is `Some`, this represents an extern function
    /// pointer type that can be passed to/from C code.
    ///
    /// # Context Requirements
    ///
    /// Function types can specify context requirements using the `using` clause:
    /// ```verum
    /// // Function type with context requirements
    /// type QueryFn is fn(Int) -> Data using [Database, Logger];
    ///
    /// // Function type with negative contexts (purity guarantee)
    /// type PureFn is fn(Int) -> Int using [!IO, !State<_>];
    /// ```
    ///
    /// # Examples
    /// ```verum
    /// // Regular Verum function type
    /// type Handler is fn(Int) -> Bool;
    ///
    /// // C-compatible function pointer type
    /// type CCallback is extern "C" fn(c_int, *mut c_void) -> c_int;
    ///
    /// // System calling convention (Windows)
    /// type WinCallback is extern "stdcall" fn(c_int) -> c_int;
    /// ```
    Function {
        params: List<Type>,
        return_type: Heap<Type>,
        /// Optional calling convention for extern function pointers
        /// None = regular Verum function type
        /// Some(C) = extern "C" fn(...) - C calling convention
        /// Some(StdCall) = extern "stdcall" fn(...) - Windows stdcall
        /// Some(FastCall) = extern "fastcall" fn(...) - Fast call convention
        /// Some(SysV64) = extern "sysv64" fn(...) - System V AMD64 ABI
        calling_convention: Maybe<CallingConvention>,
        /// Context requirements for this function type
        /// Context requirements declared via `using [Ctx]` on this function type.
        contexts: ContextList,
    },

    /// Rank-2 polymorphic function type: fn<R>(Reducer<B, R>) -> Reducer<A, R>
    ///
    /// A function type with universally quantified type parameters that scope
    /// only within the function type itself. This enables storing functions
    /// that work for ANY choice of the quantified type parameters.
    ///
    /// # Semantics
    /// The type `fn<R>(Reducer<B, R>) -> Reducer<A, R>` means:
    /// "For all types R, a function from Reducer<B, R> to Reducer<A, R>"
    ///
    /// This is essential for transducers and other higher-order abstractions
    /// that need to be polymorphic over result types.
    ///
    /// # Example
    /// ```verum
    /// // Transducer transformation type
    /// type Transform<A, B> is fn<R>(Reducer<B, R>) -> Reducer<A, R>;
    ///
    /// // Function that clones any cloneable type
    /// type Clone<T: Clone> is fn<>() -> (T, T);
    ///
    /// // With context requirements
    /// type QueryTransform<A, B> is fn<R>(Reducer<B, R>) -> Reducer<A, R> using [Database];
    /// ```
    ///
    /// # Implementation
    /// At runtime, rank-2 functions are implemented via monomorphization when
    /// the quantified parameters are known, or via dictionary passing when
    /// true polymorphism is needed.
    Rank2Function {
        /// Universally quantified type parameters local to this function type
        type_params: List<GenericParam>,
        /// Parameter types (may reference type_params)
        params: List<Type>,
        /// Return type (may reference type_params)
        return_type: Heap<Type>,
        /// Optional calling convention
        calling_convention: Maybe<CallingConvention>,
        /// Context requirements for this function type
        /// Context requirements declared via `using [Ctx]` on this function type.
        contexts: ContextList,
        /// Optional where clause constraining the quantified type parameters
        where_clause: Maybe<WhereClause>,
    },

    /// Safe reference: &T or &mut T
    Reference {
        mutable: bool,
        inner: Heap<Type>,
    },

    /// Checked reference: &checked T (compile-time proof, 0ns overhead, AOT-only)
    ///
    /// Statically verified references that require compile-time safety proof.
    /// Only available in Tier 2+ (Optimizing JIT/AOT) compilation modes.
    ///
    /// In Tier 0/1, falls back to CBGR validation for safety.
    ///
    /// # Semantics
    /// - "checked" means "statically verified", NOT "lives forever"
    /// - Requires escape analysis proof of no-escape or local-escape
    /// - Zero runtime overhead when proof succeeds
    ///
    /// # Examples
    /// ```verum
    /// fn hot_path(data: &checked List<Int>) -> Int {
    ///     // Zero-cost access, proven safe at compile time
    ///     data[0]
    /// }
    /// ```
    CheckedReference {
        mutable: bool,
        inner: Heap<Type>,
    },

    /// Unsafe reference: &unsafe T (no bounds checking)
    UnsafeReference {
        mutable: bool,
        inner: Heap<Type>,
    },

    /// Raw pointer: *const T or *mut T
    Pointer {
        mutable: bool,
        inner: Heap<Type>,
    },

    /// Volatile pointer: *volatile T or *volatile mut T
    ///
    /// Volatile pointers guarantee that reads/writes are not optimized away
    /// or reordered by the compiler. Essential for MMIO (memory-mapped I/O)
    /// and hardware register access.
    ///
    /// # Semantics
    ///
    /// - Reads are always performed (never cached or eliminated)
    /// - Writes are always performed (never deferred or eliminated)
    /// - Access order is preserved relative to other volatile accesses
    /// - No guarantee of ordering relative to non-volatile accesses
    ///   (use memory barriers for that)
    ///
    /// # Use Cases
    ///
    /// - Hardware register access (MMIO)
    /// - Shared memory with hardware devices
    /// - Memory-mapped peripherals
    /// - Interrupt status registers
    ///
    /// # Examples
    ///
    /// ```verum
    /// // Read from hardware status register
    /// let status_reg: *volatile UInt32 = 0x4000_0000 as *volatile UInt32;
    /// let value = volatile_load(status_reg);  // Never optimized away
    ///
    /// // Write to hardware control register
    /// let ctrl_reg: *volatile mut UInt32 = 0x4000_0004 as *volatile mut UInt32;
    /// volatile_store(ctrl_reg, 0x0001);  // Always performed
    /// ```
    ///
    /// # Safety
    ///
    /// Accessing volatile pointers requires unsafe code as the pointed-to
    /// hardware may have side effects on read (e.g., clearing interrupt flags).
    ///
    /// Used for memory-mapped I/O (MMIO) in embedded/systems programming where
    /// hardware registers have side effects on read/write that must not be optimized away.
    VolatilePointer {
        mutable: bool,
        inner: Heap<Type>,
    },

    /// Generic type with type arguments: List<T>
    Generic {
        base: Heap<Type>,
        args: List<GenericArg>,
    },

    /// Qualified type: <T as Protocol>::AssocType
    Qualified {
        self_ty: Heap<Type>,
        trait_ref: Path,
        assoc_name: Ident,
    },

    /// Refinement type: T{predicate}
    /// Rule 1 (Inline): Int{> 0}
    /// This is the core innovation of Verum!
    Refined {
        base: Heap<Type>,
        predicate: Heap<RefinementPredicate>,
    },

    /// Sigma-type refinement: x: T where predicate
    /// Rule 3 (Sigma-type): x: Int where x > 0
    /// Explicit name binding for dependent refinements
    /// The Sigma-type form is Rule 3 of Verum's five refinement binding rules:
    /// Rule 1 (Inline): `T{pred}` with implicit `it`
    /// Rule 2 (Lambda where): `T where |x| pred(x)`
    /// Rule 3 (Sigma): `x: T where pred(x)` -- canonical for dependent types
    /// Rule 4 (Named predicate): `T where predicate_name`
    /// Rule 5 (Bare where): `T where pred` (deprecated, use Rule 1)
    Sigma {
        name: Ident,
        base: Heap<Type>,
        predicate: Heap<Expr>,
    },

    /// Type variable for inference: _
    Inferred,

    /// Type with bounds: T where T: Protocol
    Bounded {
        base: Heap<Type>,
        bounds: List<TypeBound>,
    },

    /// Dynamic protocol object: dyn Display + Debug
    /// Uses dynamic dispatch (vtable) for runtime polymorphism.
    /// Contrasts with Bounded (impl Protocol) which uses static dispatch.
    ///
    /// Supports associated type bindings for protocols with GATs:
    /// ```verum
    /// dyn Container<Item = Int> + Display
    /// dyn Iterator<Item = String, State = Int>
    /// ```
    ///
    /// Uses dynamic dispatch via vtable. Syntax: `dyn Protocol1 + Protocol2`.
    /// Can include associated type bindings: `dyn Iterator<Item = Int>`.
    DynProtocol {
        bounds: List<TypeBound>,
        /// Optional associated type bindings (e.g., <Item = Int, State = String>)
        bindings: Maybe<List<TypeBinding>>,
    },

    /// Ownership reference: %T or %mut T
    /// Linear type that must be consumed exactly once.
    Ownership {
        mutable: bool,
        inner: Heap<Type>,
    },

    /// GenRef type: GenRef<T>
    /// Generation-aware reference for CBGR-safe lending iterators.
    /// In CBGR, lending iterators don't require lifetime annotations -- generation
    /// tracking provides safety automatically. GenRef wraps a reference with
    /// generation metadata for safe access across iterator yields.
    GenRef {
        inner: Heap<Type>,
    },

    /// Type constructor placeholder for higher-kinded types: F<_>
    /// Represents a type that takes type arguments, e.g., `type F<_>` in protocol Functor.
    /// The arity indicates how many type parameters the constructor expects.
    /// Part of the advanced protocols extension (v2.0+ planned) for encoding Functor,
    /// Monad, and similar higher-kinded abstractions.
    TypeConstructor {
        base: Heap<Type>,
        arity: usize,
    },

    /// Existential type: some T: Bound
    /// Hides concrete type behind protocol bounds, enabling information hiding
    /// without runtime cost. Similar to Rust's `impl Trait`.
    ///
    /// # Examples
    /// ```verum
    /// // Return type hides concrete iterator implementation
    /// fn make_iter() -> some I: Iterator<Item = Int> { ... }
    ///
    /// // Named existential type
    /// type Plugin is some P: PluginInterface;
    ///
    /// // Multiple bounds
    /// fn processor() -> some P: Processor + Send + Sync { ... }
    /// ```
    Existential {
        /// The existential type variable name
        name: Ident,
        /// Protocol bounds the existential must satisfy
        bounds: List<TypeBound>,
    },

    /// Associated type path: T.Item or Self.Item
    /// Enables accessing associated types from type parameters and protocol bounds.
    ///
    /// # Examples
    /// ```verum
    /// fn get_item<I: Iterator>() -> I.Item { ... }
    /// fn process<C: Collection>() -> C.Iterator.Item { ... }
    /// ```
    AssociatedType {
        /// The base type (e.g., I or Self)
        base: Heap<Type>,
        /// The associated type name (e.g., Item)
        assoc: Ident,
    },

    /// Tensor type: Tensor<T, Shape>
    ///
    /// Unified tensor type with compile-time shape tracking.
    /// Syntax: tensor<Shape...>T{elements} or Tensor<T, [d1, d2, ...]>
    ///
    /// # Examples
    /// ```verum
    /// // 1D vector: Tensor<f32, [4]>
    /// let v: Tensor<f32, [4]> = tensor<4>f32{1.0, 2.0, 3.0, 4.0};
    ///
    /// // 2D matrix: Tensor<i32, [2, 3]>
    /// let m: Tensor<i32, [2, 3]> = tensor<2, 3>i32{
    ///     {1, 2, 3},
    ///     {4, 5, 6}
    /// };
    ///
    /// // With meta parameters: Tensor<f32, [N, M]>
    /// fn matmul<N: meta usize, M: meta usize, K: meta usize>(
    ///     a: &Tensor<f32, [N, K]>,
    ///     b: &Tensor<f32, [K, M]>
    /// ) -> Tensor<f32, [N, M]>
    /// ```
    ///
    /// # Memory Layout
    /// - Row-major (C-order) by default for cache-friendliness
    /// - Column-major available for compatibility with BLAS/LAPACK
    /// - SIMD-aligned (16/32/64 bytes depending on platform)
    ///
    /// # Implementation
    /// Lowers to struct { data: [T; product(Shape)], strides: [usize; len(Shape)] }
    Tensor {
        /// Element type (f32, i32, Bool, etc.)
        element: Heap<Type>,
        /// Shape dimensions as compile-time expressions
        /// Can be:
        /// - Literal integers: [4, 8] for fixed-size tensors
        /// - Meta parameters: [N, M] for generic tensors
        /// - Expressions: [N*2, M+1] for computed shapes
        shape: List<Expr>,
        /// Memory layout (row-major vs column-major)
        /// None = row-major (default)
        layout: Maybe<TensorLayout>,
    },

    /// Capability-restricted type: T with [Capabilities]
    ///
    /// Allows defining types with restricted capabilities for fine-grained access control.
    /// This enables compile-time verification of capability requirements and automatic
    /// capability attenuation at call sites.
    ///
    /// # Subtyping Rule
    /// `T with [A, B, C] <: T with [A, B]` when the first set is a superset
    /// This means "more capabilities" is a subtype of "fewer capabilities",
    /// enabling automatic attenuation (narrowing) at call sites.
    ///
    /// # Examples
    /// ```verum
    /// // Define capability-restricted type aliases
    /// type Database.Full is Database with [Read, Write, Admin];
    /// type Database.ReadOnly is Database with [Read];
    ///
    /// // Use in function signatures
    /// fn analyze(db: Database with [Read]) -> Stats {
    ///     db.query("SELECT ...")  // OK - query only needs Read
    ///     db.execute("DELETE")    // COMPILE ERROR - Execute not in [Read]
    /// }
    ///
    /// // Automatic attenuation at call sites
    /// fn process(db: Database with [Read, Write]) {
    ///     analyze(db);  // OK - [Read, Write] ⊇ [Read], auto-attenuates
    /// }
    /// ```
    ///
    /// # Integration with Context System
    /// Works with the context system for DI:
    /// ```verum
    /// fn handler() using [Database with [Read]] {
    ///     // Only read operations available
    /// }
    /// ```
    CapabilityRestricted {
        /// The base type being restricted (e.g., Database)
        base: Heap<Type>,
        /// Set of available capabilities
        capabilities: crate::expr::CapabilitySet,
    },

    /// Anonymous record type: { field: Type, ... }
    /// Used in type expressions for anonymous structured data.
    ///
    /// # Examples
    /// ```verum
    /// // Anonymous record type in function parameter
    /// fn process(data: { name: Text, age: Int }) -> Bool { ... }
    ///
    /// // Anonymous record with refinement
    /// type ValidPoint is { x: Int, y: Int } { self.x > 0, self.y > 0 };
    /// ```
    ///
    /// # Grammar
    /// record_type = '{' , field_list , [ '|' , identifier ] , '}' ;
    ///
    /// # Row polymorphism (T1-E)
    ///
    /// When `row_var` is `Some(r)`, the record is *extensible* — it
    /// specifies some fields and leaves the rest open:
    ///
    /// ```verum
    /// // Works with any record that has at least an `x: Int` field.
    /// fn get_x<r>(p: { x: Int | r }) -> Int { p.x }
    /// ```
    ///
    /// This lowers to `Type::ExtensibleRecord { fields, row_var: Some(r) }`
    /// in the elaborated type system. A closed record (no row variable)
    /// lowers to `Type::Record`.
    Record {
        fields: List<RecordField>,
        /// Row variable capturing additional fields beyond `fields`.
        /// `Maybe::None` for a closed record.
        row_var: Maybe<Ident>,
    },

    /// Universe type: `Type`, `Type(0)`, `Type(1)`, `Type(N)`, etc.
    ///
    /// Universe types form a cumulative hierarchy preventing Girard's paradox:
    /// ```text
    /// Type(0) : Type(1) : Type(2) : ...
    /// ```
    ///
    /// `Type` is sugar for `Type(0)`. Universe polymorphism is supported
    /// through level variables in generic parameters.
    ///
    /// # Examples
    /// ```verum
    /// // Concrete universe levels
    /// type Container<T: Type(1)> is { value: T };
    ///
    /// // Universe polymorphism via level parameters
    /// fn identity<u: Level, T: Type(u)>(x: T) -> T { x }
    ///
    /// // Implicit levels (inferred)
    /// type Pair<A, B> is { fst: A, snd: B };
    /// // A: Type(u), B: Type(v), Pair: Type(max(u,v))
    /// ```
    ///
    /// # Grammar
    /// universe_type = 'Type' , [ '(' , universe_level , ')' ] ;
    /// universe_level = integer_lit | identifier ;
    Universe {
        /// The universe level. `None` means `Type` (implicitly level 0).
        /// `Some(UniverseLevelExpr::Concrete(n))` means `Type(n)`.
        /// `Some(UniverseLevelExpr::Variable(name))` means `Type(u)` for polymorphic levels.
        level: Maybe<UniverseLevelExpr>,
    },

    /// Meta type: `meta T`
    ///
    /// Represents a compile-time (meta-level) type used in dependent type programming.
    /// Meta types indicate values that exist at compile time and can be used in
    /// type-level computations.
    ///
    /// # Examples
    /// ```verum
    /// fn take<T, N: meta Nat, K: meta Nat>(
    ///     xs: SizedList<T, N>,
    ///     k: meta K        // k is a meta-level value
    /// ) -> SizedList<T, K>
    /// where K <= N { ... }
    /// ```
    Meta {
        /// The underlying type being lifted to meta level
        inner: Heap<Type>,
    },

    /// Type-level lambda: `|x| Body(x)`
    ///
    /// Used in dependent type contexts where a type is parameterized by a value.
    /// Common in sigma types: `Sigma<A, |a| B(a)>`.
    ///
    /// # Examples
    /// ```verum
    /// type Sigma<A, B: fn(A) -> Type> is (fst: A, snd: B(fst));
    /// fn example() -> Sigma<Nat, |n| List<Int, n>> { ... }
    /// ```
    TypeLambda {
        /// Parameter names
        params: List<Ident>,
        /// Body type expression (may reference params)
        body: Heap<Type>,
    },

}

impl TypeKind {
    /// Returns the display name for primitive type kinds.
    ///
    /// This is the single source of truth for converting primitive TypeKind variants
    /// to their string representation. All crates should use this instead of
    /// duplicating match arms.
    ///
    /// Returns `None` for non-primitive variants (Path, Tuple, Function, etc.)
    /// which require more context to format.
    pub fn primitive_name(&self) -> Option<&'static str> {
        match self {
            TypeKind::Unit => Some("()"),
            TypeKind::Bool => Some("Bool"),
            TypeKind::Int => Some("Int"),
            TypeKind::Float => Some("Float"),
            TypeKind::Char => Some("Char"),
            TypeKind::Text => Some("Text"),
            TypeKind::Never => Some("!"),
            TypeKind::Unknown => Some("unknown"),
            TypeKind::Inferred => Some("_"),
            _ => None,
        }
    }
}

/// Universe level expression in the AST.
///
/// Represents the level annotation on a universe type `Type(...)`.
/// This is the syntactic form; it gets lowered to `UniverseLevel` in verum_types
/// during type checking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum UniverseLevelExpr {
    /// A concrete numeric level: `Type(0)`, `Type(1)`, `Type(2)`
    Concrete(u32),
    /// A level variable: `Type(u)` where `u` is a level parameter
    Variable(Ident),
    /// Maximum of two levels: `max(u, v)` (used in inferred positions)
    Max(Heap<UniverseLevelExpr>, Heap<UniverseLevelExpr>),
    /// Successor of a level: `u + 1` (used in inferred positions)
    Succ(Heap<UniverseLevelExpr>),
}

/// Tensor memory layout strategy.
///
/// The memory layout affects cache locality and SIMD efficiency:
/// - RowMajor: Last dimension is contiguous (C-order, NumPy default)
/// - ColumnMajor: First dimension is contiguous (Fortran-order, BLAS/LAPACK)
///
/// # Performance implications
/// - RowMajor: Better for row-wise operations, natural for SIMD vectorization
/// - ColumnMajor: Better for column-wise operations, required for BLAS interop
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TensorLayout {
    /// Row-major (C-order): last dimension varies fastest
    /// Memory: [row0_col0, row0_col1, row0_col2, row1_col0, ...]
    /// Strides for shape [M, N]: [N, 1]
    RowMajor,

    /// Column-major (Fortran-order): first dimension varies fastest
    /// Memory: [row0_col0, row1_col0, row2_col0, row0_col1, ...]
    /// Strides for shape [M, N]: [1, M]
    ColumnMajor,
}

/// A refinement predicate attached to a type.
///
/// Refinement types are Verum's unique value proposition:
/// Rule 1 (Inline): type Positive is Int{> 0}
/// Rule 2 (Lambda): type Positive is Int where |x| x > 0
/// Rule 3 (Sigma): type Positive is x: Int where x > 0
/// Rule 4 (Named): type Email is Text where is_email
/// Rule 5 (Bare where - deprecated): type Positive is Int where it > 0
///
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefinementPredicate {
    /// The predicate expression
    pub expr: Expr,
    /// Optional explicit binding name for lambda-style (|x| expr) or sigma-type (x: T where expr)
    /// None = implicit 'it' binding (Rule 1, 5)
    /// Some(name) = explicit binding (Rule 2, 3)
    pub binding: Maybe<Ident>,
    /// The span of the predicate
    pub span: Span,
}

impl RefinementPredicate {
    pub fn new(expr: Expr, span: Span) -> Self {
        Self {
            expr,
            binding: Maybe::None,
            span,
        }
    }

    pub fn with_binding(expr: Expr, binding: Maybe<Ident>, span: Span) -> Self {
        Self {
            expr,
            binding,
            span,
        }
    }
}

/// A generic argument (type or const expression).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GenericArg {
    /// Type argument: List<Int>
    Type(Type),
    /// Const argument: Array<T, 10>
    Const(Expr),
    /// Lifetime argument (for future expansion)
    Lifetime(Lifetime),
    /// Associated type binding: Deref<Target=Vec<Int>>
    Binding(TypeBinding),
}

/// A type bound (protocol constraint).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeBound {
    pub kind: TypeBoundKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TypeBoundKind {
    /// Protocol bound: T: Trait
    Protocol(Path),
    /// Equality bound: T = ConcreteType
    Equality(Type),
    /// Negative protocol bound: T: !Trait
    /// Used for specialization mutual exclusion -- asserts a type does NOT
    /// implement a protocol, enabling non-overlapping specialization branches.
    NegativeProtocol(Path),
    /// Generic protocol bound: T: Iterator<Item = U> or T: Container<Element = V>
    /// Protocol bound with generic type arguments, including associated type bindings.
    ///
    /// # Examples
    /// ```verum
    /// type Iter: Iterator<Item = Self.Item>;
    /// fn foo<I: IntoIterator<Item = Int>>(iter: I);
    /// ```
    GenericProtocol(Type),
    /// Associated type bound: I.Item: Display
    /// Constrains an associated type to satisfy certain bounds.
    ///
    /// # Examples
    /// ```verum
    /// where type I.Item: Display + Clone
    /// where type C.Iterator.Item: Num
    /// ```
    AssociatedTypeBound {
        /// The type that has the associated type (e.g., I)
        type_path: Path,
        /// The associated type name (e.g., Item)
        assoc_name: Ident,
        /// The bounds the associated type must satisfy
        bounds: List<TypeBound>,
    },
    /// Associated type equality: I.Item = T
    /// Requires an associated type to equal a specific type.
    ///
    /// # Examples
    /// ```verum
    /// where type I.Item = Int
    /// where type I.Item = J.Item
    /// ```
    AssociatedTypeEquality {
        /// The type that has the associated type (e.g., I)
        type_path: Path,
        /// The associated type name (e.g., Item)
        assoc_name: Ident,
        /// The type that the associated type must equal
        eq_type: Type,
    },
}

/// A type binding for associated types in dyn protocol objects.
///
/// Used to bind associated types when using dynamic protocol objects:
/// ```verum
/// dyn Container<Item = Int> + Display
/// dyn Iterator<Item = String, State = Int>
/// ```
///
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeBinding {
    /// The name of the associated type (e.g., "Item")
    pub name: Ident,
    /// The concrete type to bind it to (e.g., Int, String)
    pub ty: Type,
    /// The span of the entire binding
    pub span: Span,
}

impl TypeBinding {
    pub fn new(name: Ident, ty: Type, span: Span) -> Self {
        Self { name, ty, span }
    }
}

/// A generic parameter in a type or function definition.
///
/// Supports both explicit and implicit parameters:
/// - Explicit: `<T>`, `<N: meta Nat>` - must be provided at call site
/// - Implicit: `<{T}>`, `<{N: meta Nat}>` - inferred from usage context
///
/// Implicit parameters use `{...}` syntax (e.g., `<{A: Type}>`) and are inferred
/// from usage context rather than requiring explicit specification at call sites.
///
/// # Examples
/// ```verum
/// // Explicit type parameter
/// fn id<T>(x: T) -> T = x
/// let y = id<Int>(42)  // Must specify T
///
/// // Implicit type parameter - inferred from argument
/// fn singleton<{A: Type}>(x: A) -> List<A, 1: meta usize> =
///     Cons(x, Nil)
/// let v = singleton(42)  // A inferred as Int
///
/// // Implicit value parameter
/// fn complex<{n: meta Nat}>(x: Int) -> List<Int, n: meta Nat> = ...
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenericParam {
    pub kind: GenericParamKind,
    /// Whether this parameter is implicit (inferred from context).
    /// Implicit parameters use `{...}` syntax in declarations.
    pub is_implicit: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GenericParamKind {
    /// Type parameter: T
    Type {
        name: Ident,
        bounds: List<TypeBound>,
        default: Maybe<Type>,
    },
    /// Higher-kinded type parameter: F<_>, F<_, _>
    /// Enables abstraction over type constructors like Functor, Monad.
    ///
    /// # Examples
    /// ```verum
    /// fn map<F<_>: Functor, A, B>(fa: F<A>, f: fn(A) -> B) -> F<B>
    /// type Bifunctor is protocol { type F<_, _>; ... }
    /// ```
    HigherKinded {
        /// The name of the HKT parameter (e.g., F)
        name: Ident,
        /// Number of type placeholders (arity)
        /// 1 for F<_>, 2 for F<_, _>, etc.
        arity: usize,
        /// Protocol bounds the type constructor must satisfy
        bounds: List<TypeBound>,
    },
    /// Const parameter: const N: usize (deprecated - use Meta instead)
    Const { name: Ident, ty: Type },
    /// Meta parameter: N: meta usize or N: meta usize{> 0}
    /// The `meta` keyword marks compile-time value parameters in the unified meta-system.
    /// Replaces const generics -- all compile-time computation is unified under `meta`.
    Meta {
        name: Ident,
        ty: Type,
        refinement: Maybe<Heap<Expr>>,
    },
    /// Lifetime parameter (for future expansion)
    Lifetime { name: Lifetime },

    /// Context parameter: using C
    ///
    /// Enables higher-order functions to propagate contexts from callbacks.
    /// The context variable can be used in function signatures to indicate
    /// that the function's context requirements depend on the callback.
    ///
    /// # Examples
    /// ```verum
    /// // Context-polymorphic map function
    /// fn map<T, U, using C>(
    ///     iter: Iterator<Item = T>,
    ///     f: fn(T) -> U using C
    /// ) -> MapIter<T, U, C> using C {
    ///     MapIter { iter, f }
    /// }
    ///
    /// // Usage - context inferred from callback
    /// data.iter().map(|x| Database.lookup(x.id))  // C = [Database]
    /// ```
    Context {
        /// The name of the context variable (e.g., C)
        name: Ident,
    },

    /// Universe level parameter: `u: Level`
    ///
    /// Enables universe polymorphism — functions and types that work across
    /// multiple universe levels. The `Level` keyword marks a generic parameter
    /// as a universe level variable rather than a type variable.
    ///
    /// # Examples
    /// ```verum
    /// // Universe-polymorphic identity
    /// fn identity<u: Level, T: Type(u)>(x: T) -> T { x }
    ///
    /// // Universe-polymorphic container
    /// type Container<u: Level, T: Type(u)> is { value: T };
    /// ```
    Level {
        /// The name of the level variable (e.g., u)
        name: Ident,
    },

    /// Kind-annotated type parameter: `F: Type -> Type`
    ///
    /// An explicit kind annotation expressing that `F` is a type constructor of
    /// the given kind, using arrow notation. This is the spec-required alternative
    /// to the `F<_>` placeholder syntax.
    ///
    /// # Grammar
    /// ```ebnf
    /// kind_annotated_param = identifier , ':' , kind_expr ;
    /// kind_expr = 'Type' , [ '->' , kind_expr ] | '(' , kind_expr , ')' ;
    /// ```
    ///
    /// # Examples
    /// ```verum
    /// type Functor<F: Type -> Type> is protocol {
    ///     fn map<A, B>(fa: F<A>, f: fn(A) -> B) -> F<B>;
    /// };
    ///
    /// type Bifunctor<F: Type -> Type -> Type> is protocol {
    ///     fn bimap<A, B, C, D>(fab: F<A, B>, f: fn(A) -> C, g: fn(B) -> D) -> F<C, D>;
    /// };
    /// ```
    KindAnnotated {
        /// The name of the type constructor parameter (e.g., F, M)
        name: Ident,
        /// The kind annotation, e.g. `Type -> Type`
        kind: KindAnnotation,
        /// Optional protocol bounds on the type constructor (e.g., `F: Type -> Type + Functor`)
        bounds: List<TypeBound>,
    },
}

/// A kind annotation for use in `F: Type -> Type` generic parameter syntax.
///
/// This mirrors `verum_types::poly_kinds::Kind` but lives in the AST crate so
/// that the parser can represent kind annotations without depending on the type
/// system crate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum KindAnnotation {
    /// The base kind `Type` — the kind of all value-level concrete types.
    Type,
    /// An arrow kind `K1 -> K2` — the kind of a type constructor.
    Arrow(Box<KindAnnotation>, Box<KindAnnotation>),
}

impl KindAnnotation {
    /// Compute the arity (number of type arguments) this kind expects.
    ///
    /// `Type` has arity 0, `Type -> Type` has arity 1,
    /// `Type -> Type -> Type` has arity 2, etc.
    pub fn arity(&self) -> usize {
        match self {
            KindAnnotation::Type => 0,
            KindAnnotation::Arrow(_, rhs) => 1 + rhs.arity(),
        }
    }
}

impl std::fmt::Display for KindAnnotation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KindAnnotation::Type => write!(f, "Type"),
            KindAnnotation::Arrow(lhs, rhs) => {
                match lhs.as_ref() {
                    KindAnnotation::Arrow(_, _) => write!(f, "({}) -> {}", lhs, rhs),
                    _ => write!(f, "{} -> {}", lhs, rhs),
                }
            }
        }
    }
}

/// A path to a type or value (e.g., std.collections.Vec).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Path {
    pub segments: SmallVec<[PathSegment; 4]>,
    pub span: Span,
}

impl Path {
    pub fn new(segments: List<PathSegment>, span: Span) -> Self {
        // Convert List to SmallVec directly
        Self {
            segments: segments.into_iter().collect(),
            span,
        }
    }

    pub fn single(ident: Ident) -> Self {
        let span = ident.span;
        Self {
            segments: smallvec::smallvec![PathSegment::Name(ident)],
            span,
        }
    }

    /// Create a Path from a single Ident.
    /// This is an alias for `Path::single` for better API ergonomics.
    ///
    /// # Examples
    /// ```
    /// use verum_ast::{Path, Ident};
    /// use verum_ast::span::Span;
    ///
    /// let ident = Ident::new("foo", Span::dummy());
    /// let path = Path::from_ident(ident);
    /// assert!(path.is_single());
    /// ```
    pub fn from_ident(ident: Ident) -> Self {
        Self::single(ident)
    }

    pub fn is_single(&self) -> bool {
        self.segments.len() == 1
    }

    pub fn as_ident(&self) -> Option<&Ident> {
        if let [PathSegment::Name(ident)] = self.segments.as_slice() {
            Some(ident)
        } else {
            None
        }
    }

    /// Returns the name of the last `Name` segment in the path, or `""` if none.
    pub fn last_segment_name(&self) -> &str {
        for seg in self.segments.iter().rev() {
            if let PathSegment::Name(ident) = seg {
                return &ident.name;
            }
        }
        ""
    }
}

impl Spanned for Path {
    fn span(&self) -> Span {
        self.span
    }
}

impl std::fmt::Display for Path {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, segment) in self.segments.iter().enumerate() {
            if i > 0 {
                write!(f, "::")?;
            }
            match segment {
                PathSegment::Name(ident) => write!(f, "{}", ident.name)?,
                PathSegment::SelfValue => write!(f, "self")?,
                PathSegment::Super => write!(f, "super")?,
                PathSegment::Cog => write!(f, "cog")?,
                PathSegment::Relative => write!(f, ".")?,
            }
        }
        Ok(())
    }
}

/// A segment in a path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PathSegment {
    /// Regular name segment
    Name(Ident),
    /// Self keyword
    SelfValue,
    /// Super keyword
    Super,
    /// Cog root
    Cog,
    /// Relative path marker (leading dot)
    Relative,
}

/// An identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Ident {
    pub name: Text,
    pub span: Span,
}

impl Ident {
    pub fn new(name: impl Into<Text>, span: Span) -> Self {
        Self {
            name: name.into(),
            span,
        }
    }

    pub fn as_str(&self) -> &str {
        self.name.as_str()
    }
}

impl Spanned for Ident {
    fn span(&self) -> Span {
        self.span
    }
}

impl std::fmt::Display for Ident {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

/// A lifetime (for future lifetime tracking).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Lifetime {
    pub name: Text,
    pub span: Span,
}

/// A where clause containing predicates.
///
/// In v6.0-BALANCED, there are two separate where clause types:
/// 1. Generic where clause (where type T: Protocol) - for type constraints
/// 2. Meta where clause (where meta N > 0) - for meta-parameter refinements
///
/// These are represented as separate optional fields in declarations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WhereClause {
    pub predicates: List<WherePredicate>,
    pub span: Span,
}

impl WhereClause {
    pub fn new(predicates: List<WherePredicate>, span: Span) -> Self {
        Self { predicates, span }
    }

    pub fn empty(span: Span) -> Self {
        Self {
            predicates: List::new(),
            span,
        }
    }
}

/// A predicate in a where clause.
///
/// The `where` keyword serves four distinct purposes in v6.0-BALANCED:
/// 1. `where type T: Protocol` - Generic type constraints
/// 2. `where meta N > 0` - Meta-parameter refinements
/// 3. `where value it > 0` - Value refinements (explicit)
/// 4. `where ensures result >= 0` - Postconditions
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WherePredicate {
    pub kind: WherePredicateKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WherePredicateKind {
    /// Generic type constraint: where type T: Protocol
    /// Example: fn sort<T>(list: List<T>) where type T: Ord
    Type { ty: Type, bounds: List<TypeBound> },

    /// Meta-parameter refinement: where meta N > 0
    /// Example: type Matrix<N: meta usize> where meta N > 0
    Meta { constraint: Expr },

    /// Value refinement: where value it > 0
    /// Example: type Positive is Int where value it > 0
    /// Note: The 'value' keyword is optional for backward compatibility
    Value { predicate: Expr },

    /// Postcondition: where ensures result >= 0
    /// Example: fn abs(x: Int) -> Int where ensures result >= 0
    Ensures { postcondition: Expr },
}

// Forward declaration - actual Expr is in expr.rs
// We use a simple placeholder here to avoid circular dependencies
// The real Expr will be used by other modules
pub use crate::expr::Expr;
