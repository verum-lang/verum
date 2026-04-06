//! SyntaxKind enum defining all node and token types in the Verum syntax tree.
//!
//! Design: u16 for compact storage in green nodes. Values 0-255 are tokens,
//! values 256+ are composite nodes.
//!
//! SyntaxKind Design: u16 representation for compact storage in green nodes.
//! Values 0-255 are tokens (keywords, punctuation, literals, identifiers).
//! Values 256+ are composite nodes (expressions, statements, declarations).
//! Follows industry convention from Roslyn (C#) and rust-analyzer.

// The SCREAMING_CASE naming follows industry convention (Roslyn, rust-analyzer)
#![allow(non_camel_case_types)]

use core::fmt;

/// All syntax node and token kinds in Verum.
///
/// This enum is the backbone of the lossless syntax tree. Every node and token
/// in the tree has an associated SyntaxKind.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum SyntaxKind {
    // ============================================================
    // TOKENS (0-255)
    // ============================================================

    // === Reserved Keywords (0-9) ===
    /// `let` keyword - Reserved
    LET_KW = 0,
    /// `fn` keyword - Reserved
    FN_KW = 1,
    /// `is` keyword - Reserved (unified type syntax)
    IS_KW = 2,

    // === Primary Keywords (10-29) ===
    /// `type` keyword
    TYPE_KW = 10,
    /// `where` keyword
    WHERE_KW = 11,
    /// `using` keyword
    USING_KW = 12,
    /// `match` keyword
    MATCH_KW = 13,
    /// `mount` keyword
    MOUNT_KW = 14,

    // === Control Flow Keywords (30-49) ===
    /// `if` keyword
    IF_KW = 30,
    /// `else` keyword
    ELSE_KW = 31,
    /// `while` keyword
    WHILE_KW = 32,
    /// `for` keyword
    FOR_KW = 33,
    /// `loop` keyword
    LOOP_KW = 34,
    /// `break` keyword
    BREAK_KW = 35,
    /// `continue` keyword
    CONTINUE_KW = 36,
    /// `return` keyword
    RETURN_KW = 37,
    /// `yield` keyword
    YIELD_KW = 38,

    // === Async/Context Keywords (50-69) ===
    /// `async` keyword
    ASYNC_KW = 50,
    /// `await` keyword
    AWAIT_KW = 51,
    /// `spawn` keyword
    SPAWN_KW = 52,
    /// `select` keyword
    SELECT_KW = 53,
    /// `defer` keyword
    DEFER_KW = 54,
    /// `errdefer` keyword
    ERRDEFER_KW = 55,
    /// `try` keyword
    TRY_KW = 56,
    /// `throw` keyword
    THROW_KW = 57,
    /// `throws` keyword
    THROWS_KW = 58,
    /// `recover` keyword
    RECOVER_KW = 59,
    /// `finally` keyword
    FINALLY_KW = 60,
    /// `nursery` keyword - structured concurrency block that spawns child tasks
    /// and waits for all to complete. Supports timeout, error behavior (fail-fast
    /// or collect-all), and cancellation. Child task failures propagate to parent.
    NURSERY_KW = 61,

    // === Modifier Keywords (70-89) ===
    /// `pub` keyword
    PUB_KW = 70,
    /// `public` keyword
    PUBLIC_KW = 71,
    /// `internal` keyword
    INTERNAL_KW = 72,
    /// `protected` keyword
    PROTECTED_KW = 73,
    /// `private` keyword
    PRIVATE_KW = 74,
    /// `mut` keyword
    MUT_KW = 75,
    /// `const` keyword
    CONST_KW = 76,
    /// `static` keyword
    STATIC_KW = 77,
    /// `unsafe` keyword
    UNSAFE_KW = 78,
    /// `meta` keyword
    META_KW = 79,
    /// `pure` keyword
    PURE_KW = 80,
    /// `affine` keyword
    AFFINE_KW = 81,
    /// `linear` keyword
    LINEAR_KW = 82,
    /// `quote` keyword
    QUOTE_KW = 83,
    /// `stage` keyword
    STAGE_KW = 84,
    /// `lift` keyword
    LIFT_KW = 85,
    /// `volatile` keyword
    VOLATILE_KW = 86,

    // === Module/Type Keywords (90-109) ===
    /// `module` keyword
    MODULE_KW = 90,
    /// `implement` keyword
    IMPLEMENT_KW = 91,
    /// `protocol` keyword
    PROTOCOL_KW = 92,
    /// `extends` keyword
    EXTENDS_KW = 93,
    /// `context` keyword
    CONTEXT_KW = 94,
    /// `provide` keyword
    PROVIDE_KW = 95,
    /// `ffi` keyword
    FFI_KW = 96,
    /// `extern` keyword
    EXTERN_KW = 97,
    /// `stream` keyword
    STREAM_KW = 98,
    /// `tensor` keyword
    TENSOR_KW = 99,
    /// `set` keyword - Set comprehensions
    SET_KW = 100,
    /// `gen` keyword - Generator expressions
    GEN_KW = 101,

    // === Reference/Value Keywords (110-119) ===
    /// `self` value
    SELF_VALUE_KW = 110,
    /// `Self` type
    SELF_TYPE_KW = 111,
    /// `super` keyword
    SUPER_KW = 112,
    /// `cog` keyword
    COG_KW = 113,
    /// `ref` keyword
    REF_KW = 114,
    /// `move` keyword
    MOVE_KW = 115,
    /// `as` keyword
    AS_KW = 116,
    /// `in` keyword
    IN_KW = 117,
    /// `checked` keyword
    CHECKED_KW = 118,

    // === Verification Keywords (120-139) ===
    /// `requires` keyword
    REQUIRES_KW = 120,
    /// `ensures` keyword
    ENSURES_KW = 121,
    /// `invariant` keyword
    INVARIANT_KW = 122,
    /// `decreases` keyword
    DECREASES_KW = 123,
    /// `result` keyword
    RESULT_KW = 124,
    /// `view` keyword
    VIEW_KW = 125,
    /// `pattern` keyword - active pattern declarations
    PATTERN_KW = 126,
    /// `with` keyword - capability-restricted types. Syntax: `Type with [Capabilities]`
    /// Enables fine-grained capability attenuation, e.g., `Database with [Read]`
    /// restricts a database handle to read-only operations at the type level.
    /// Capability subtyping: `T with [Read, Write]` <: `T with [Read]`.
    WITH_KW = 127,
    /// `unknown` keyword - top type for safe dynamic typing. Every type T satisfies
    /// T <: unknown. Used with `Data` type for runtime-typed values (JSON, APIs).
    /// NOT an unsafe escape hatch -- requires explicit pattern matching to extract values.
    UNKNOWN_KW = 128,
    /// `typeof` keyword - runtime type introspection. Returns the TypeId of a value
    /// at runtime. Used in type guard patterns: `if x is Int { ... }` or
    /// `match typeof(x) { ... }`. Integrates with the Data type for dynamic dispatch.
    TYPEOF_KW = 129,

    // === Proof Keywords (140-169) ===
    /// `theorem` keyword
    THEOREM_KW = 140,
    /// `axiom` keyword
    AXIOM_KW = 141,
    /// `lemma` keyword
    LEMMA_KW = 142,
    /// `corollary` keyword
    COROLLARY_KW = 143,
    /// `proof` keyword
    PROOF_KW = 144,
    /// `calc` keyword
    CALC_KW = 145,
    /// `have` keyword
    HAVE_KW = 146,
    /// `show` keyword
    SHOW_KW = 147,
    /// `suffices` keyword
    SUFFICES_KW = 148,
    /// `obtain` keyword
    OBTAIN_KW = 149,
    /// `by` keyword
    BY_KW = 150,
    /// `induction` keyword
    INDUCTION_KW = 151,
    /// `cases` keyword
    CASES_KW = 152,
    /// `contradiction` keyword
    CONTRADICTION_KW = 153,
    /// `trivial` keyword
    TRIVIAL_KW = 154,
    /// `assumption` keyword
    ASSUMPTION_KW = 155,
    /// `simp` keyword
    SIMP_KW = 156,
    /// `ring` keyword
    RING_KW = 157,
    /// `field` keyword
    FIELD_KW = 158,
    /// `omega` keyword
    OMEGA_KW = 159,
    /// `auto` keyword
    AUTO_KW = 160,
    /// `blast` keyword
    BLAST_KW = 161,
    /// `smt` keyword
    SMT_KW = 162,
    /// `qed` keyword
    QED_KW = 163,
    /// `forall` keyword
    FORALL_KW = 164,
    /// `exists` keyword
    EXISTS_KW = 165,
    /// `cofix` keyword
    COFIX_KW = 166,
    /// `implies` keyword (logical implication for proofs)
    IMPLIES_KW = 167,

    // === Boolean Literals (170-179) ===
    /// `true` literal
    TRUE_KW = 170,
    /// `false` literal
    FALSE_KW = 171,
    /// `None` variant
    NONE_KW = 172,
    /// `Some` variant
    SOME_KW = 173,
    /// `Ok` variant
    OK_KW = 174,
    /// `Err` variant
    ERR_KW = 175,

    // === Punctuation (180-209) ===
    /// `(`
    L_PAREN = 180,
    /// `)`
    R_PAREN = 181,
    /// `[`
    L_BRACKET = 182,
    /// `]`
    R_BRACKET = 183,
    /// `{`
    L_BRACE = 184,
    /// `}`
    R_BRACE = 185,
    /// `<`
    L_ANGLE = 186,
    /// `>`
    R_ANGLE = 187,
    /// `;`
    SEMICOLON = 188,
    /// `,`
    COMMA = 189,
    /// `:`
    COLON = 190,
    /// `::`
    COLON_COLON = 191,
    /// `.`
    DOT = 192,
    /// `..`
    DOT_DOT = 193,
    /// `..=`
    DOT_DOT_EQ = 194,
    /// `@`
    AT = 195,
    /// `#`
    HASH = 196,
    /// `?`
    QUESTION = 197,
    /// `?.`
    QUESTION_DOT = 198,
    /// `??`
    QUESTION_QUESTION = 199,
    /// `??!`
    QUESTION_QUESTION_BANG = 200,
    /// `_`
    UNDERSCORE = 201,

    // === Operators (210-239) ===
    /// `+`
    PLUS = 210,
    /// `-`
    MINUS = 211,
    /// `*`
    STAR = 212,
    /// `/`
    SLASH = 213,
    /// `%`
    PERCENT = 214,
    /// `**`
    STAR_STAR = 215,
    /// `=`
    EQ = 216,
    /// `==`
    EQ_EQ = 217,
    /// `!=`
    BANG_EQ = 218,
    // Note: LT and GT are the same as L_ANGLE and R_ANGLE (use L_ANGLE/R_ANGLE)
    /// `<=`
    LT_EQ = 219,
    /// `>=`
    GT_EQ = 220,
    /// `&&`
    AMP_AMP = 221,
    /// `||`
    PIPE_PIPE = 222,
    /// `!`
    BANG = 223,
    /// `&`
    AMP = 224,
    /// `|`
    PIPE = 225,
    /// `^`
    CARET = 226,
    /// `~`
    TILDE = 227,
    /// `<<`
    LT_LT = 228,
    /// `>>`
    GT_GT = 229,
    /// `->`
    ARROW = 230,
    /// `=>`
    FAT_ARROW = 231,
    /// `|>`
    PIPE_GT = 232,
    /// `>>`
    GT_GT_COMPOSE = 233,
    /// `<<`
    LT_LT_COMPOSE = 234,

    // === Compound Assignment (235-244) ===
    /// `+=`
    PLUS_EQ = 235,
    /// `-=`
    MINUS_EQ = 236,
    /// `*=`
    STAR_EQ = 237,
    /// `/=`
    SLASH_EQ = 238,
    /// `%=`
    PERCENT_EQ = 239,
    /// `&=`
    AMP_EQ = 240,
    /// `|=`
    PIPE_EQ = 241,
    /// `^=`
    CARET_EQ = 242,
    /// `<<=`
    LT_LT_EQ = 243,
    /// `>>=`
    GT_GT_EQ = 244,

    // === Identifiers and Literals (245-254) ===
    /// Identifier
    IDENT = 245,
    /// Integer literal
    INT_LITERAL = 246,
    /// Float literal
    FLOAT_LITERAL = 247,
    /// String literal
    STRING_LITERAL = 248,
    /// Char literal
    CHAR_LITERAL = 249,
    /// Byte string literal (b"...")
    BYTE_STRING_LITERAL = 250,
    /// Interpolated string literal
    INTERPOLATED_STRING = 251,
    /// Tagged literal (e.g., `rx#"..."`)
    TAGGED_LITERAL = 252,
    /// Hex color literal (e.g., `#FF5733`)
    HEX_COLOR = 253,
    /// Contract literal
    CONTRACT_LITERAL = 254,
    /// Whitespace (spaces, tabs)
    WHITESPACE = 255,

    // ============================================================
    // Note: Token range is 0-255, composite nodes are 256+
    // NEWLINE moved to composite nodes for space
    // ============================================================

    // ============================================================
    // COMPOSITE NODES (256+)
    // ============================================================

    // === Trivia Nodes (256-260) ===
    /// Newline (LF or CRLF)
    NEWLINE = 256,
    /// Line comment `// ...`
    LINE_COMMENT = 257,
    /// Block comment `/* ... */`
    BLOCK_COMMENT = 258,
    /// Doc comment `/// ...`
    DOC_COMMENT = 259,
    /// Inner doc comment `//! ...`
    INNER_DOC_COMMENT = 260,

    // === Special (261-265) ===
    /// Error node - wraps unparseable content
    ERROR = 261,
    /// End of file marker
    EOF = 262,
    /// Tombstone - placeholder for abandoned markers
    TOMBSTONE = 263,
    /// Root of the syntax tree
    SOURCE_FILE = 264,

    // === Declarations (265-294) ===
    /// Function definition
    FN_DEF = 265,
    /// Type definition
    TYPE_DEF = 266,
    /// Protocol definition
    PROTOCOL_DEF = 267,
    /// Implementation block
    IMPL_BLOCK = 268,
    /// Context definition
    CONTEXT_DEF = 269,
    /// Context group definition
    CONTEXT_GROUP_DEF = 270,
    /// Const definition
    CONST_DEF = 271,
    /// Static definition
    STATIC_DEF = 272,
    /// Mount statement
    MOUNT_STMT = 273,
    /// Module definition
    MODULE_DEF = 274,
    /// Meta definition
    META_DEF = 275,
    /// FFI declaration
    FFI_DECL = 276,
    /// Attribute
    ATTRIBUTE = 277,
    /// Attribute item (attribute + item)
    ATTR_ITEM = 278,

    // === Proof Declarations (280-289) ===
    /// Theorem declaration
    THEOREM_DEF = 280,
    /// Axiom declaration
    AXIOM_DEF = 281,
    /// Lemma declaration
    LEMMA_DEF = 282,
    /// Corollary declaration
    COROLLARY_DEF = 283,
    /// Proof block
    PROOF_BLOCK = 284,
    /// Calc chain
    CALC_BLOCK = 285,

    // === Statements (295-319) ===
    /// Let statement
    LET_STMT = 295,
    /// Expression statement
    EXPR_STMT = 296,
    /// Return statement
    RETURN_STMT = 297,
    /// Break statement
    BREAK_STMT = 298,
    /// Continue statement
    CONTINUE_STMT = 299,
    /// Yield statement
    YIELD_STMT = 300,
    /// Defer statement
    DEFER_STMT = 301,
    /// Errdefer statement
    ERRDEFER_STMT = 302,
    /// Provide statement
    PROVIDE_STMT = 303,
    /// Throw statement
    THROW_STMT = 304,

    // === Expressions (320-399) ===
    /// Literal expression
    LITERAL_EXPR = 320,
    /// Path expression (e.g., `foo::bar`)
    PATH_EXPR = 321,
    /// Binary expression
    BINARY_EXPR = 322,
    /// Unary expression (prefix)
    PREFIX_EXPR = 323,
    /// Postfix expression (e.g., `x?`, `x.await`)
    POSTFIX_EXPR = 324,
    /// Call expression
    CALL_EXPR = 325,
    /// Method call expression
    METHOD_CALL_EXPR = 326,
    /// Field expression
    FIELD_EXPR = 327,
    /// Index expression
    INDEX_EXPR = 328,
    /// If expression
    IF_EXPR = 329,
    /// Match expression
    MATCH_EXPR = 330,
    /// Block expression
    BLOCK_EXPR = 331,
    /// Closure expression
    CLOSURE_EXPR = 332,
    /// Async expression
    ASYNC_EXPR = 333,
    /// Await expression
    AWAIT_EXPR = 334,
    /// Spawn expression
    SPAWN_EXPR = 335,
    /// Select expression
    SELECT_EXPR = 336,
    /// Pipeline expression
    PIPELINE_EXPR = 337,
    /// Try expression
    TRY_EXPR = 338,
    /// Tuple expression
    TUPLE_EXPR = 339,
    /// Array expression
    ARRAY_EXPR = 340,
    /// Record expression (struct literal)
    RECORD_EXPR = 341,
    /// Range expression
    RANGE_EXPR = 342,
    /// Cast expression (`as`)
    CAST_EXPR = 343,
    /// Reference expression (`&x`)
    REF_EXPR = 344,
    /// Dereference expression (`*x`)
    DEREF_EXPR = 345,
    /// Parenthesized expression
    PAREN_EXPR = 346,
    /// Loop expression
    LOOP_EXPR = 347,
    /// While expression
    WHILE_EXPR = 348,
    /// For expression
    FOR_EXPR = 349,
    /// For-await expression
    FOR_AWAIT_EXPR = 350,
    /// Stream expression
    STREAM_EXPR = 351,
    /// Refinement expression
    REFINEMENT_EXPR = 352,
    /// Recover expression
    RECOVER_EXPR = 353,
    /// Map expression (dictionary literal)
    MAP_EXPR = 354,
    /// Set expression
    SET_EXPR = 355,
    /// Map entry (key: value pair)
    MAP_ENTRY = 356,
    /// Tensor expression
    TENSOR_EXPR = 357,
    /// Comprehension expression (list comprehension)
    COMPREHENSION_EXPR = 358,
    /// Stream comprehension expression
    STREAM_COMPREHENSION_EXPR = 359,
    /// Comprehension clause (for/if)
    COMPREHENSION_CLAUSE = 360,
    /// Throw expression
    THROW_EXPR = 361,
    /// Is expression (pattern test)
    IS_EXPR = 362,
    /// Quote expression (metaprogramming)
    QUOTE_EXPR = 363,
    /// Splice expression (metaprogramming)
    SPLICE_EXPR = 364,
    /// Generator expression
    GENERATOR_EXPR = 365,
    /// Yield expression
    YIELD_EXPR = 366,
    /// Forall expression (universal quantifier)
    FORALL_EXPR = 367,
    /// Exists expression (existential quantifier)
    EXISTS_EXPR = 368,

    // === Types (400-449) ===
    /// Path type (e.g., `foo::Bar`)
    PATH_TYPE = 400,
    /// Reference type (`&T`, `&mut T`)
    REFERENCE_TYPE = 401,
    /// Pointer type (`*T`, `*mut T`)
    POINTER_TYPE = 402,
    /// Function type (`fn(A) -> B`)
    FUNCTION_TYPE = 403,
    /// Tuple type (`(A, B, C)`)
    TUPLE_TYPE = 404,
    /// Array type (`[T; N]`)
    ARRAY_TYPE = 405,
    /// Slice type (`[T]`)
    SLICE_TYPE = 406,
    /// Refined type (with `where` clause)
    REFINED_TYPE = 407,
    /// Generic type (with type arguments)
    GENERIC_TYPE = 408,
    /// Infer type (`_`)
    INFER_TYPE = 409,
    /// Never type (`!`)
    NEVER_TYPE = 410,
    /// GenRef type
    GENREF_TYPE = 411,
    /// Parenthesized type
    PAREN_TYPE = 412,
    /// Type property access (e.g., `T.size`)
    TYPE_PROPERTY = 413,
    /// Dynamic type (dyn TraitA + TraitB)
    DYNAMIC_TYPE = 414,
    /// Associated type bindings
    ASSOC_TYPE_BINDINGS = 415,
    /// Type binding (Item = T)
    TYPE_BINDING = 416,
    /// GAT parameters
    GAT_PARAMS = 417,
    /// GAT where clause
    GAT_WHERE_CLAUSE = 418,
    /// GAT constraint
    GAT_CONSTRAINT = 419,
    /// Union type (A | B)
    UNION_TYPE = 420,
    /// Intersection type (A & B)
    INTERSECTION_TYPE = 421,

    // === Patterns (450-479) ===
    /// Wildcard pattern (`_`)
    WILDCARD_PAT = 450,
    /// Identifier pattern
    IDENT_PAT = 451,
    /// Literal pattern
    LITERAL_PAT = 452,
    /// Tuple pattern
    TUPLE_PAT = 453,
    /// Record pattern
    RECORD_PAT = 454,
    /// Variant pattern (constructor pattern)
    VARIANT_PAT = 455,
    /// Or pattern (`P1 | P2`)
    OR_PAT = 456,
    /// Range pattern (`0..10`)
    RANGE_PAT = 457,
    /// Reference pattern (`&p`)
    REF_PAT = 458,
    /// Rest pattern (`..`)
    REST_PAT = 459,
    /// Bind pattern (`name @ pattern`)
    BIND_PAT = 460,
    /// Parenthesized pattern
    PAREN_PAT = 461,
    /// Slice pattern
    SLICE_PAT = 462,
    /// Guard pattern (pattern with `if` or `where`)
    GUARD_PAT = 463,

    // === Auxiliary Nodes (480-549) ===
    /// Parameter list
    PARAM_LIST = 480,
    /// Parameter
    PARAM = 481,
    /// Self parameter
    SELF_PARAM = 482,
    /// Argument list
    ARG_LIST = 483,
    /// Argument
    ARG = 484,
    /// Generic parameters
    GENERIC_PARAMS = 485,
    /// Generic parameter
    GENERIC_PARAM = 486,
    /// Type parameter
    TYPE_PARAM = 487,
    /// Meta parameter (const generic)
    META_PARAM = 488,
    /// Generic arguments
    GENERIC_ARGS = 489,
    /// Type argument
    TYPE_ARG = 490,
    /// Where clause
    WHERE_CLAUSE = 491,
    /// Where predicate
    WHERE_PRED = 492,
    /// Type bound
    TYPE_BOUND = 493,
    /// Bound list
    BOUND_LIST = 494,
    /// Using clause (context requirements)
    USING_CLAUSE = 495,
    /// Context path
    CONTEXT_PATH = 496,
    /// Block (statements in braces)
    BLOCK = 497,
    /// Match arm
    MATCH_ARM = 498,
    /// Match arm list
    MATCH_ARM_LIST = 499,
    /// Field list
    FIELD_LIST = 500,
    /// Record field
    RECORD_FIELD = 501,
    /// Field definition (in type)
    FIELD_DEF = 502,
    /// Variant definition
    VARIANT_DEF = 503,
    /// Variant list
    VARIANT_LIST = 504,
    /// Visibility modifier
    VISIBILITY = 505,
    /// Path segment
    PATH_SEGMENT = 506,
    /// Path
    PATH = 507,
    /// Mount tree
    MOUNT_TREE = 508,
    /// Mount list
    MOUNT_LIST = 509,
    /// Label (for loops)
    LABEL = 510,
    /// Throws clause
    THROWS_CLAUSE = 511,
    /// Ensures clause
    ENSURES_CLAUSE = 512,
    /// Requires clause
    REQUIRES_CLAUSE = 513,
    /// Invariant clause
    INVARIANT_CLAUSE = 514,
    /// Decreases clause
    DECREASES_CLAUSE = 515,
    /// Recover match arms
    RECOVER_ARMS = 516,
    /// Recover closure
    RECOVER_CLOSURE = 517,

    // === Protocol Items (550-569) ===
    /// Protocol item
    PROTOCOL_ITEM = 550,
    /// Protocol function (method signature)
    PROTOCOL_FN = 551,
    /// Associated type
    ASSOC_TYPE = 552,
    /// Associated const
    ASSOC_CONST = 553,

    // === Implementation Items (570-589) ===
    /// Implementation item
    IMPL_ITEM = 570,
    /// Function implementation
    IMPL_FN = 571,
    /// Type alias implementation
    IMPL_TYPE = 572,
    /// Const implementation
    IMPL_CONST = 573,
    /// Default implementation marker
    DEFAULT_IMPL = 574,

    // === Select Expression Parts (590-599) ===
    /// Select arm
    SELECT_ARM = 590,
    /// Select arm list
    SELECT_ARM_LIST = 591,
    /// Default arm
    DEFAULT_ARM = 592,

    // === FFI Items (600-619) ===
    /// FFI function
    FFI_FN = 600,
    /// FFI type
    FFI_TYPE = 601,
    /// FFI const
    FFI_CONST = 602,
    /// FFI block
    FFI_BLOCK = 603,

    // Ensure we have a known maximum
    #[doc(hidden)]
    __LAST = 999,
}

impl SyntaxKind {
    /// Returns true if this kind represents a token (not a composite node).
    #[inline]
    pub const fn is_token(self) -> bool {
        (self as u16) < 256
    }

    /// Returns true if this kind represents trivia (whitespace, comments).
    #[inline]
    pub const fn is_trivia(self) -> bool {
        matches!(
            self,
            SyntaxKind::WHITESPACE
                | SyntaxKind::NEWLINE
                | SyntaxKind::LINE_COMMENT
                | SyntaxKind::BLOCK_COMMENT
                | SyntaxKind::DOC_COMMENT
                | SyntaxKind::INNER_DOC_COMMENT
        )
    }

    /// Returns true if this kind represents a keyword.
    #[inline]
    pub const fn is_keyword(self) -> bool {
        (self as u16) < 180
    }

    /// Returns true if this kind represents a punctuation mark.
    #[inline]
    pub const fn is_punct(self) -> bool {
        let v = self as u16;
        v >= 180 && v <= 244
    }

    /// Returns true if this kind represents a literal.
    #[inline]
    pub const fn is_literal(self) -> bool {
        matches!(
            self,
            SyntaxKind::INT_LITERAL
                | SyntaxKind::FLOAT_LITERAL
                | SyntaxKind::STRING_LITERAL
                | SyntaxKind::CHAR_LITERAL
                | SyntaxKind::BYTE_STRING_LITERAL
                | SyntaxKind::INTERPOLATED_STRING
                | SyntaxKind::TAGGED_LITERAL
                | SyntaxKind::HEX_COLOR
                | SyntaxKind::CONTRACT_LITERAL
                | SyntaxKind::TRUE_KW
                | SyntaxKind::FALSE_KW
        )
    }

    /// Returns true if this kind can start an expression.
    #[inline]
    pub const fn can_start_expr(self) -> bool {
        self.is_literal()
            || matches!(
                self,
                SyntaxKind::IDENT
                    | SyntaxKind::SELF_VALUE_KW
                    | SyntaxKind::SELF_TYPE_KW
                    | SyntaxKind::L_PAREN
                    | SyntaxKind::L_BRACKET
                    | SyntaxKind::L_BRACE
                    | SyntaxKind::IF_KW
                    | SyntaxKind::MATCH_KW
                    | SyntaxKind::LOOP_KW
                    | SyntaxKind::WHILE_KW
                    | SyntaxKind::FOR_KW
                    | SyntaxKind::ASYNC_KW
                    | SyntaxKind::UNSAFE_KW
                    | SyntaxKind::META_KW
                    | SyntaxKind::QUOTE_KW
                    | SyntaxKind::STREAM_KW
                    | SyntaxKind::SET_KW
                    | SyntaxKind::GEN_KW
                    | SyntaxKind::RETURN_KW
                    | SyntaxKind::THROW_KW
                    | SyntaxKind::BREAK_KW
                    | SyntaxKind::CONTINUE_KW
                    | SyntaxKind::MINUS
                    | SyntaxKind::BANG
                    | SyntaxKind::TILDE
                    | SyntaxKind::AMP
                    | SyntaxKind::PERCENT
                    | SyntaxKind::STAR
                    | SyntaxKind::PIPE
            )
    }

    /// Returns true if this kind can start a type.
    #[inline]
    pub const fn can_start_type(self) -> bool {
        matches!(
            self,
            SyntaxKind::IDENT
                | SyntaxKind::SELF_TYPE_KW
                | SyntaxKind::L_PAREN
                | SyntaxKind::L_BRACKET
                | SyntaxKind::AMP
                | SyntaxKind::STAR
                | SyntaxKind::FN_KW
                | SyntaxKind::UNDERSCORE
                | SyntaxKind::BANG
        )
    }

    /// Returns true if this kind can start an item/declaration.
    #[inline]
    pub const fn can_start_item(self) -> bool {
        matches!(
            self,
            SyntaxKind::FN_KW
                | SyntaxKind::TYPE_KW
                | SyntaxKind::IMPLEMENT_KW
                | SyntaxKind::PROTOCOL_KW
                | SyntaxKind::CONTEXT_KW
                | SyntaxKind::CONST_KW
                | SyntaxKind::STATIC_KW
                | SyntaxKind::MOUNT_KW
                | SyntaxKind::MODULE_KW
                | SyntaxKind::META_KW
                | SyntaxKind::FFI_KW
                | SyntaxKind::AT
                | SyntaxKind::PUB_KW
                | SyntaxKind::PUBLIC_KW
                | SyntaxKind::INTERNAL_KW
                | SyntaxKind::PROTECTED_KW
                | SyntaxKind::ASYNC_KW
                | SyntaxKind::UNSAFE_KW
                | SyntaxKind::PURE_KW
                | SyntaxKind::AFFINE_KW
                | SyntaxKind::THEOREM_KW
                | SyntaxKind::AXIOM_KW
                | SyntaxKind::LEMMA_KW
                | SyntaxKind::COROLLARY_KW
        )
    }

    /// Returns true if this is an ERROR node.
    #[inline]
    pub const fn is_error(self) -> bool {
        matches!(self, SyntaxKind::ERROR)
    }

    /// Get the human-readable name of this kind.
    pub const fn name(self) -> &'static str {
        match self {
            SyntaxKind::LET_KW => "let",
            SyntaxKind::FN_KW => "fn",
            SyntaxKind::IS_KW => "is",
            SyntaxKind::TYPE_KW => "type",
            SyntaxKind::WHERE_KW => "where",
            SyntaxKind::USING_KW => "using",
            SyntaxKind::MATCH_KW => "match",
            SyntaxKind::MOUNT_KW => "mount",
            SyntaxKind::IF_KW => "if",
            SyntaxKind::ELSE_KW => "else",
            SyntaxKind::WHILE_KW => "while",
            SyntaxKind::FOR_KW => "for",
            SyntaxKind::LOOP_KW => "loop",
            SyntaxKind::BREAK_KW => "break",
            SyntaxKind::CONTINUE_KW => "continue",
            SyntaxKind::RETURN_KW => "return",
            SyntaxKind::YIELD_KW => "yield",
            SyntaxKind::ASYNC_KW => "async",
            SyntaxKind::AWAIT_KW => "await",
            SyntaxKind::SPAWN_KW => "spawn",
            SyntaxKind::SELECT_KW => "select",
            SyntaxKind::DEFER_KW => "defer",
            SyntaxKind::ERRDEFER_KW => "errdefer",
            SyntaxKind::TRY_KW => "try",
            SyntaxKind::THROW_KW => "throw",
            SyntaxKind::THROWS_KW => "throws",
            SyntaxKind::RECOVER_KW => "recover",
            SyntaxKind::FINALLY_KW => "finally",
            SyntaxKind::NURSERY_KW => "nursery",
            SyntaxKind::PUB_KW => "pub",
            SyntaxKind::PUBLIC_KW => "public",
            SyntaxKind::INTERNAL_KW => "internal",
            SyntaxKind::PROTECTED_KW => "protected",
            SyntaxKind::PRIVATE_KW => "private",
            SyntaxKind::MUT_KW => "mut",
            SyntaxKind::CONST_KW => "const",
            SyntaxKind::STATIC_KW => "static",
            SyntaxKind::VOLATILE_KW => "volatile",
            SyntaxKind::UNSAFE_KW => "unsafe",
            SyntaxKind::META_KW => "meta",
            SyntaxKind::PURE_KW => "pure",
            SyntaxKind::AFFINE_KW => "affine",
            SyntaxKind::LINEAR_KW => "linear",
            SyntaxKind::QUOTE_KW => "quote",
            SyntaxKind::STAGE_KW => "stage",
            SyntaxKind::LIFT_KW => "lift",
            SyntaxKind::MODULE_KW => "module",
            SyntaxKind::IMPLEMENT_KW => "implement",
            SyntaxKind::PROTOCOL_KW => "protocol",
            SyntaxKind::EXTENDS_KW => "extends",
            SyntaxKind::CONTEXT_KW => "context",
            SyntaxKind::PROVIDE_KW => "provide",
            SyntaxKind::FFI_KW => "ffi",
            SyntaxKind::EXTERN_KW => "extern",
            SyntaxKind::STREAM_KW => "stream",
            SyntaxKind::TENSOR_KW => "tensor",
            SyntaxKind::SET_KW => "set",
            SyntaxKind::GEN_KW => "gen",
            SyntaxKind::SELF_VALUE_KW => "self",
            SyntaxKind::SELF_TYPE_KW => "Self",
            SyntaxKind::SUPER_KW => "super",
            SyntaxKind::COG_KW => "cog",
            SyntaxKind::REF_KW => "ref",
            SyntaxKind::MOVE_KW => "move",
            SyntaxKind::AS_KW => "as",
            SyntaxKind::IN_KW => "in",
            SyntaxKind::CHECKED_KW => "checked",
            SyntaxKind::REQUIRES_KW => "requires",
            SyntaxKind::ENSURES_KW => "ensures",
            SyntaxKind::INVARIANT_KW => "invariant",
            SyntaxKind::DECREASES_KW => "decreases",
            SyntaxKind::RESULT_KW => "result",
            SyntaxKind::VIEW_KW => "view",
            SyntaxKind::PATTERN_KW => "pattern",
            SyntaxKind::WITH_KW => "with",
            SyntaxKind::UNKNOWN_KW => "unknown",
            SyntaxKind::TYPEOF_KW => "typeof",
            SyntaxKind::THEOREM_KW => "theorem",
            SyntaxKind::AXIOM_KW => "axiom",
            SyntaxKind::LEMMA_KW => "lemma",
            SyntaxKind::COROLLARY_KW => "corollary",
            SyntaxKind::PROOF_KW => "proof",
            SyntaxKind::CALC_KW => "calc",
            SyntaxKind::HAVE_KW => "have",
            SyntaxKind::SHOW_KW => "show",
            SyntaxKind::SUFFICES_KW => "suffices",
            SyntaxKind::OBTAIN_KW => "obtain",
            SyntaxKind::BY_KW => "by",
            SyntaxKind::INDUCTION_KW => "induction",
            SyntaxKind::CASES_KW => "cases",
            SyntaxKind::CONTRADICTION_KW => "contradiction",
            SyntaxKind::TRIVIAL_KW => "trivial",
            SyntaxKind::ASSUMPTION_KW => "assumption",
            SyntaxKind::SIMP_KW => "simp",
            SyntaxKind::RING_KW => "ring",
            SyntaxKind::FIELD_KW => "field",
            SyntaxKind::OMEGA_KW => "omega",
            SyntaxKind::AUTO_KW => "auto",
            SyntaxKind::BLAST_KW => "blast",
            SyntaxKind::SMT_KW => "smt",
            SyntaxKind::QED_KW => "qed",
            SyntaxKind::FORALL_KW => "forall",
            SyntaxKind::EXISTS_KW => "exists",
            SyntaxKind::COFIX_KW => "cofix",
            SyntaxKind::IMPLIES_KW => "implies",
            SyntaxKind::TRUE_KW => "true",
            SyntaxKind::FALSE_KW => "false",
            SyntaxKind::NONE_KW => "None",
            SyntaxKind::SOME_KW => "Some",
            SyntaxKind::OK_KW => "Ok",
            SyntaxKind::ERR_KW => "Err",
            SyntaxKind::L_PAREN => "(",
            SyntaxKind::R_PAREN => ")",
            SyntaxKind::L_BRACKET => "[",
            SyntaxKind::R_BRACKET => "]",
            SyntaxKind::L_BRACE => "{",
            SyntaxKind::R_BRACE => "}",
            SyntaxKind::L_ANGLE => "<",
            SyntaxKind::R_ANGLE => ">",
            SyntaxKind::SEMICOLON => ";",
            SyntaxKind::COMMA => ",",
            SyntaxKind::COLON => ":",
            SyntaxKind::COLON_COLON => "::",
            SyntaxKind::DOT => ".",
            SyntaxKind::DOT_DOT => "..",
            SyntaxKind::DOT_DOT_EQ => "..=",
            SyntaxKind::AT => "@",
            SyntaxKind::HASH => "#",
            SyntaxKind::QUESTION => "?",
            SyntaxKind::QUESTION_DOT => "?.",
            SyntaxKind::QUESTION_QUESTION => "??",
            SyntaxKind::QUESTION_QUESTION_BANG => "??!",
            SyntaxKind::UNDERSCORE => "_",
            SyntaxKind::PLUS => "+",
            SyntaxKind::MINUS => "-",
            SyntaxKind::STAR => "*",
            SyntaxKind::SLASH => "/",
            SyntaxKind::PERCENT => "%",
            SyntaxKind::STAR_STAR => "**",
            SyntaxKind::EQ => "=",
            SyntaxKind::EQ_EQ => "==",
            SyntaxKind::BANG_EQ => "!=",
            SyntaxKind::LT_EQ => "<=",
            SyntaxKind::GT_EQ => ">=",
            SyntaxKind::AMP_AMP => "&&",
            SyntaxKind::PIPE_PIPE => "||",
            SyntaxKind::BANG => "!",
            SyntaxKind::AMP => "&",
            SyntaxKind::PIPE => "|",
            SyntaxKind::CARET => "^",
            SyntaxKind::TILDE => "~",
            SyntaxKind::LT_LT => "<<",
            SyntaxKind::GT_GT => ">>",
            SyntaxKind::ARROW => "->",
            SyntaxKind::FAT_ARROW => "=>",
            SyntaxKind::PIPE_GT => "|>",
            SyntaxKind::GT_GT_COMPOSE => ">> (compose)",
            SyntaxKind::LT_LT_COMPOSE => "<< (compose)",
            SyntaxKind::PLUS_EQ => "+=",
            SyntaxKind::MINUS_EQ => "-=",
            SyntaxKind::STAR_EQ => "*=",
            SyntaxKind::SLASH_EQ => "/=",
            SyntaxKind::PERCENT_EQ => "%=",
            SyntaxKind::AMP_EQ => "&=",
            SyntaxKind::PIPE_EQ => "|=",
            SyntaxKind::CARET_EQ => "^=",
            SyntaxKind::LT_LT_EQ => "<<=",
            SyntaxKind::GT_GT_EQ => ">>=",
            SyntaxKind::IDENT => "identifier",
            SyntaxKind::INT_LITERAL => "integer literal",
            SyntaxKind::FLOAT_LITERAL => "float literal",
            SyntaxKind::STRING_LITERAL => "string literal",
            SyntaxKind::CHAR_LITERAL => "char literal",
            SyntaxKind::BYTE_STRING_LITERAL => "byte string literal",
            SyntaxKind::INTERPOLATED_STRING => "interpolated string",
            SyntaxKind::TAGGED_LITERAL => "tagged literal",
            SyntaxKind::HEX_COLOR => "hex color",
            SyntaxKind::CONTRACT_LITERAL => "contract literal",
            SyntaxKind::WHITESPACE => "whitespace",
            SyntaxKind::NEWLINE => "newline",
            SyntaxKind::LINE_COMMENT => "line comment",
            SyntaxKind::BLOCK_COMMENT => "block comment",
            SyntaxKind::DOC_COMMENT => "doc comment",
            SyntaxKind::INNER_DOC_COMMENT => "inner doc comment",
            SyntaxKind::ERROR => "error",
            SyntaxKind::EOF => "end of file",
            SyntaxKind::TOMBSTONE => "tombstone",
            SyntaxKind::SOURCE_FILE => "source file",
            SyntaxKind::FN_DEF => "function definition",
            SyntaxKind::TYPE_DEF => "type definition",
            SyntaxKind::PROTOCOL_DEF => "protocol definition",
            SyntaxKind::IMPL_BLOCK => "implementation block",
            SyntaxKind::CONTEXT_DEF => "context definition",
            SyntaxKind::CONTEXT_GROUP_DEF => "context group definition",
            SyntaxKind::CONST_DEF => "const definition",
            SyntaxKind::STATIC_DEF => "static definition",
            SyntaxKind::MOUNT_STMT => "mount statement",
            SyntaxKind::MODULE_DEF => "module definition",
            SyntaxKind::META_DEF => "meta definition",
            SyntaxKind::FFI_DECL => "FFI declaration",
            SyntaxKind::ATTRIBUTE => "attribute",
            SyntaxKind::ATTR_ITEM => "attributed item",
            _ => "unknown",
        }
    }
}

impl fmt::Debug for SyntaxKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

impl fmt::Display for SyntaxKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

impl From<u16> for SyntaxKind {
    fn from(raw: u16) -> SyntaxKind {
        // SAFETY: We ensure all values are valid SyntaxKind variants by
        // checking raw <= __LAST. SyntaxKind is #[repr(u16)] so transmute is safe.
        if raw <= SyntaxKind::__LAST as u16 {
            unsafe { std::mem::transmute::<u16, SyntaxKind>(raw) }
        } else {
            SyntaxKind::ERROR
        }
    }
}

impl From<SyntaxKind> for u16 {
    fn from(kind: SyntaxKind) -> u16 {
        kind as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_token() {
        assert!(SyntaxKind::LET_KW.is_token());
        assert!(SyntaxKind::IDENT.is_token());
        assert!(SyntaxKind::WHITESPACE.is_token());
        assert!(!SyntaxKind::FN_DEF.is_token());
        assert!(!SyntaxKind::SOURCE_FILE.is_token());
    }

    #[test]
    fn test_is_trivia() {
        assert!(SyntaxKind::WHITESPACE.is_trivia());
        assert!(SyntaxKind::NEWLINE.is_trivia());
        assert!(SyntaxKind::LINE_COMMENT.is_trivia());
        assert!(SyntaxKind::BLOCK_COMMENT.is_trivia());
        assert!(!SyntaxKind::LET_KW.is_trivia());
        assert!(!SyntaxKind::IDENT.is_trivia());
    }

    #[test]
    fn test_is_keyword() {
        assert!(SyntaxKind::LET_KW.is_keyword());
        assert!(SyntaxKind::FN_KW.is_keyword());
        assert!(SyntaxKind::ASYNC_KW.is_keyword());
        assert!(!SyntaxKind::L_PAREN.is_keyword());
        assert!(!SyntaxKind::IDENT.is_keyword());
    }

    #[test]
    fn test_conversion() {
        let kind = SyntaxKind::LET_KW;
        let raw: u16 = kind.into();
        let back: SyntaxKind = raw.into();
        assert_eq!(kind, back);
    }

    #[test]
    fn test_can_start_item() {
        assert!(SyntaxKind::FN_KW.can_start_item());
        assert!(SyntaxKind::TYPE_KW.can_start_item());
        assert!(SyntaxKind::PUB_KW.can_start_item());
        assert!(SyntaxKind::AT.can_start_item());
        assert!(!SyntaxKind::LET_KW.can_start_item());
        assert!(!SyntaxKind::IDENT.can_start_item());
    }
}
