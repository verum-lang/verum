//! Error types and recovery strategies for the parser.
//!
//! This module provides standalone error types for parsing errors using
//! native recursive descent parsing.
//!
//! # Error Codes
//!
//! The parser uses a comprehensive error code system for categorized diagnostics:
//!
//! - **E001-E009**: Lexer/literal errors
//!   - E001: Unterminated character literal
//!   - E002: Invalid escape sequence
//!   - E003: Invalid number literal
//!   - E004: Empty character literal
//!   - E005: Invalid interpolation syntax
//!   - E006: Unknown token/character
//! - **E010-E019**: Statement/attribute errors
//!   - E010: Missing semicolon
//!   - E011: Unclosed attribute parenthesis
//!   - E012: Invalid attribute arguments
//!   - E013: Missing attribute name
//!   - E014: Invalid nested attribute
//!   - E015: Invalid empty cfg attribute
//!   - E016: Invalid empty requires clause
//!   - E017: Invalid empty ensures clause
//!   - E018: Unexpected token
//!   - E019: Missing block after control flow
//! - **E020-E029**: Proof/theorem errors
//!   - E020: Invalid theorem declaration
//!   - E021: Missing theorem name
//!   - E022: Invalid lemma declaration
//!   - E023: Unclosed forall quantifier
//!   - E024: Unclosed exists quantifier
//!   - E025: Invalid proof keyword
//!   - E026: Invalid assert expression
//!   - E027: Invalid assume expression
//!   - E028: Malformed tactic
//!   - E029: Proof block not terminated
//! - **E030-E039**: Function declaration errors
//!   - E030: Missing function name
//!   - E031: Missing function parameter list
//!   - E032: Missing function body
//!   - E033: Invalid function visibility
//!   - E034: Duplicate function modifier
//!   - E035: Invalid function parameter
//!   - E036: Missing parameter type
//!   - E037: Invalid return type syntax
//!   - E038: Invalid where clause syntax
//!   - E039: Invalid using clause syntax
//! - **E040-E049**: Type definition errors
//!   - E040: Invalid throws clause syntax
//!   - E041: Missing generic closing bracket
//!   - E042: Empty generic parameters
//!   - E043: Missing type name
//!   - E044: Missing type 'is' keyword
//!   - E045: Missing type body
//!   - E046: Invalid record field syntax
//!   - E047: Missing field type
//!   - E048: Invalid variant syntax
//!   - E049: Duplicate field name
//! - **E050-E059**: Protocol/implement errors
//!   - E050: Invalid generic constraint
//!   - E051: Missing protocol opening brace
//!   - E052: Invalid protocol method
//!   - E053: Invalid refinement syntax
//!   - E054: Missing impl type
//!   - E055: Missing 'for' in trait impl
//!   - E056: Invalid impl method
//!   - E057: Missing impl opening brace
//!   - E058: Missing context name
//!   - E059: Missing context body
//! - **E060-E069**: Context/module/const errors
//!   - E060: Invalid context method
//!   - E061: Missing module name
//!   - E062: Missing module opening brace
//!   - E063: Invalid link syntax
//!   - E064: Invalid pub use syntax
//!   - E065: Missing const type
//!   - E066: Missing const value
//!   - E067: Missing static type
//!   - E068: Invalid const/static expression
//!   - E069: Duplicate generic parameter name
//! - **E070-E079**: Type syntax errors
//!   - E070: Unclosed array type
//!   - E071: Array type missing size
//!   - E072: Array type with negative size
//!   - E073: Array type with double semicolon
//!   - E074: Array missing element type
//!   - E075: Unclosed capability list
//!   - E076: Empty capability list
//!   - E077: Capability syntax without 'with' keyword
//!   - E078: Unclosed refinement type
//!   - E079: Refinement without base type
//! - **E080-E089**: Reference/pointer type errors
//!   - E080: Invalid integer type suffix
//!   - E081: Unclosed type constraint generic
//!   - E082: Empty generic type arguments
//!   - E083: Double comma in capability list
//!   - E084: Trailing comma in capability list
//!   - E085: Double opening angle bracket
//!   - E086: Invalid double ampersand in reference
//!   - E087: Reference without type
//!   - E088: Double checked in reference
//!   - E089: Conflicting reference modifiers
//! - **E090-E099**: Function type errors
//!   - E090: Rank-2 function missing parameter list
//!   - E091: Unclosed function parameter list
//!   - E092: Function type missing return type
//!   - E093: Wrong arrow operator (=> instead of ->)
//!   - E094: Unclosed throws clause in function type
//!   - E095: Using clause without context list
//!   - E096: Async keyword in wrong position
//!   - E097: Unclosed tuple type
//!   - E098: Single element tuple invalid
//!   - E099: Unit type with content
//! - **E0A0-E0AF**: Exception/control flow errors
//!   - E0A0: Throw without expression
//!   - E0A1: Finally clause without block
//!   - E0A2: Recover with malformed closure
//! - **E0B0-E0BF**: Expression syntax errors
//!   - E0B0: Generic type args unclosed angle
//!   - E0B1: Turbofish missing type
//!   - E0B2: Tuple index invalid literal
//! - **E0C0-E0CF**: Quantifier/typeof errors
//!   - E0C0: Tagged literal missing string
//!   - E0C1: Typeof expression without argument
//!   - E0C2: Forall missing dot before body

use std::fmt;
use verum_ast::{FileId, Span};
pub use verum_common::error::VerumError;
use verum_common::{List, Text, global_span_to_line_col};
use verum_lexer::{Token, TokenKind};

/// Standard error codes for the Verum parser.
/// These codes follow the VCS specification for diagnostic output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum ErrorCode {
    // E001-E009: Lexer/literal errors
    UnterminatedChar = 0x001,
    InvalidEscape = 0x002,
    InvalidNumber = 0x003,
    EmptyChar = 0x004,
    InvalidInterpolation = 0x005,
    UnknownToken = 0x006,

    // E010-E019: Statement/attribute errors
    MissingSemicolon = 0x010,
    UnclosedAttribute = 0x011,
    InvalidAttributeArgs = 0x012,
    MissingAttributeName = 0x013,
    InvalidNestedAttribute = 0x014,
    InvalidEmptyCfg = 0x015,
    InvalidEmptyRequires = 0x016,
    InvalidEmptyEnsures = 0x017,
    UnexpectedToken = 0x018,
    MissingBlock = 0x019,

    // E020-E029: Proof/theorem errors
    InvalidTheorem = 0x020,
    MissingTheoremName = 0x021,
    InvalidLemma = 0x022,
    UnclosedForall = 0x023,
    UnclosedExists = 0x024,
    InvalidProofKeyword = 0x025,
    InvalidAssert = 0x026,
    InvalidAssume = 0x027,
    MalformedTactic = 0x028,
    ProofNotTerminated = 0x029,

    // E030-E039: Function declaration errors
    MissingFnName = 0x030,
    MissingFnParams = 0x031,
    MissingFnBody = 0x032,
    InvalidFnVisibility = 0x033,
    DuplicateFnModifier = 0x034,
    InvalidFnParam = 0x035,
    MissingParamType = 0x036,
    InvalidReturnType = 0x037,
    InvalidWhereClauseSyntax = 0x038,
    InvalidUsingClauseSyntax = 0x039,

    // E040-E049: Type definition errors
    InvalidThrowsClause = 0x040,
    MissingGenericClose = 0x041,
    EmptyGenericParams = 0x042,
    MissingTypeName = 0x043,
    MissingTypeIs = 0x044,
    MissingTypeBody = 0x045,
    InvalidRecordField = 0x046,
    MissingFieldType = 0x047,
    InvalidVariantSyntax = 0x048,
    DuplicateFieldName = 0x049,

    // E050-E059: Protocol/implement errors
    InvalidGenericConstraint = 0x050,
    MissingProtocolBrace = 0x051,
    InvalidProtocolMethod = 0x052,
    InvalidRefinementSyntax = 0x053,
    MissingImplType = 0x054,
    MissingImplFor = 0x055,
    InvalidImplMethod = 0x056,
    MissingImplBrace = 0x057,
    MissingContextName = 0x058,
    MissingContextBody = 0x059,

    // E060-E069: Context/module/const errors
    InvalidContextMethod = 0x060,
    MissingModuleName = 0x061,
    MissingModuleBrace = 0x062,
    InvalidMountSyntax = 0x063,
    InvalidPubUseSyntax = 0x064,
    MissingConstType = 0x065,
    MissingConstValue = 0x066,
    MissingStaticType = 0x067,
    InvalidConstExpr = 0x068,
    DuplicateGenericParam = 0x069,

    // E070-E079: Type syntax errors
    UnclosedArrayType = 0x070,
    ArrayMissingSize = 0x071,
    ArrayNegativeSize = 0x072,
    ArrayDoubleSemicolon = 0x073,
    ArrayMissingElement = 0x074,
    UnclosedCapability = 0x075,
    EmptyCapability = 0x076,
    CapabilityNoWith = 0x077,
    UnclosedRefinement = 0x078,
    RefinementNoBase = 0x079,

    // E080-E089: Reference/pointer type errors
    InvalidIntSuffix = 0x080,
    UnclosedConstraintGeneric = 0x081,
    EmptyGenericArgs = 0x082,
    DoubleCommaCapability = 0x083,
    TrailingCommaCapability = 0x084,
    DoubleAngleBracket = 0x085,
    DoubleAmpersandRef = 0x086,
    RefWithoutType = 0x087,
    DoubleCheckedRef = 0x088,
    ConflictingRefModifiers = 0x089,

    // E090-E099: Function type errors
    Rank2MissingParams = 0x090,
    UnclosedFnParams = 0x091,
    FnTypeMissingReturn = 0x092,
    WrongArrowOperator = 0x093,
    UnclosedThrows = 0x094,
    UsingWithoutContext = 0x095,
    AsyncWrongPosition = 0x096,
    UnclosedTupleType = 0x097,
    SingleElementTuple = 0x098,
    UnitWithContent = 0x099,

    // E0A0-E0AF: Exception/control flow errors
    ThrowNoExpression = 0x0A0,
    FinallyNoBlock = 0x0A1,
    RecoverMalformedClosure = 0x0A2,
    InvalidAsyncBlock = 0x0A3,
    InvalidAwaitExpr = 0x0A4,
    InvalidSelectArm = 0x0A5,
    InvalidSpawnExpr = 0x0A6,
    MissingChannelOp = 0x0A7,
    UnclosedSelect = 0x0A8,
    InvalidBreak = 0x0A9,
    InvalidContinue = 0x0AA,
    InvalidReturn = 0x0AB,
    InvalidYield = 0x0AC,

    // E0B0-E0BF: Expression syntax errors
    UnclosedGenericArgs = 0x0B0,
    TurbofishMissingType = 0x0B1,
    InvalidTupleIndex = 0x0B2,
    InvalidFieldAccess = 0x0B3,
    InvalidMethodCall = 0x0B4,
    InvalidIndexExpr = 0x0B5,
    InvalidCallArgs = 0x0B6,
    InvalidClosure = 0x0B7,
    InvalidMatch = 0x0B8,
    InvalidIf = 0x0B9,
    InvalidFor = 0x0BA,
    InvalidWhile = 0x0BB,
    InvalidLoop = 0x0BC,
    InvalidRange = 0x0BD,
    InvalidBinaryOp = 0x0BE,
    InvalidUnaryOp = 0x0BF,

    // E0C0-E0CF: Quantifier/typeof errors
    TaggedLiteralMissing = 0x0C0,
    TypeofNoArg = 0x0C1,
    ForallMissingDot = 0x0C2,
    ExistsMissingDot = 0x0C3,
    InvalidComprehension = 0x0C4,
    InvalidPipeline = 0x0C5,
    InvalidTryExpr = 0x0C6,
    InvalidDefer = 0x0C7,
    InvalidProvide = 0x0C8,
    InvalidLetPattern = 0x0C9,

    // E0E0-E0EF: Rust syntax used in Verum (migration helpers)
    // E0E0: Rust keyword used instead of Verum equivalent
    RustKeywordUsed = 0x0E0,
    // E0E1: Rust type name used instead of Verum semantic type
    RustTypeUsed = 0x0E1,
    // E0E2: Rust macro syntax (!) used instead of Verum syntax
    RustMacroSyntax = 0x0E2,

    // E0D0-E0DF: Grammar violation errors (strict grammar enforcement)
    // E0D0: Trailing separator without following element
    TrailingSeparator = 0x0D0,
    // E0D1: Empty construct where grammar requires content
    EmptyConstruct = 0x0D1,
    // E0D2: Duplicate clause (e.g., multiple where clauses)
    DuplicateClause = 0x0D2,
    // E0D3: Invalid splice ($ without identifier or expression)
    InvalidSplice = 0x0D3,
    // E0D4: Missing required block expression
    MissingBlockExpr = 0x0D4,
    // E0D5: Empty shape parameters in tensor
    EmptyShapeParams = 0x0D5,

    // Pattern-specific errors (map to E070-E089 for VCS compatibility)
    // These have different hex values but map to same string codes as type errors
    // E070: Invalid @ binding pattern
    PatternInvalidAt = 0x170,
    // E071: Invalid identifier in pattern
    PatternInvalidIdentifier = 0x171,
    // E072: Invalid rest/spread pattern position
    PatternInvalidRest = 0x172,
    // E073: Invalid mut pattern
    PatternInvalidMut = 0x173,
    // E074: Empty tuple pattern with trailing comma
    PatternEmptyTuple = 0x174,
    // E075: Invalid active pattern arguments
    PatternInvalidActiveArgs = 0x175,
    // E076: Invalid field pattern syntax
    PatternInvalidField = 0x176,
    // E077: Duplicate field in pattern
    PatternDuplicateField = 0x177,
    // E078: Nested or-pattern without parentheses
    PatternNestedOr = 0x178,
    // E079: Or-pattern with inconsistent bindings
    PatternOrBinding = 0x179,
    // E080: Invalid pattern type annotation
    PatternInvalidType = 0x17A,
    // E081: Invalid slice pattern syntax
    PatternInvalidSlice = 0x17B,
    // E082: Invalid unicode pattern
    PatternInvalidUnicode = 0x17C,
    // E083: Invalid variant pattern arguments
    PatternInvalidVariantArgs = 0x17D,
    // E084: Invalid and-pattern (combination)
    PatternInvalidAnd = 0x17E,
    // E085: Trailing pipe in pattern
    PatternTrailingPipe = 0x17F,
    // E086: Invalid guard expression
    PatternInvalidGuard = 0x180,
    // E087: Invalid match arm syntax
    PatternInvalidMatchArm = 0x181,
    // E088: Invalid let pattern (in let-else, etc.)
    PatternInvalidLet = 0x182,
    // E089: Empty or-pattern (consecutive pipes)
    PatternEmptyOr = 0x183,

    // Statement-level errors (VCS specification)
    // These map to E011, E012, E013, E040-E049 as specified by VCS tests
    // Using distinct hex values (0x2xx) to avoid conflicts

    // E011: Unclosed block statement
    StmtUnclosedBlock = 0x211,
    // E012: Unclosed function call parenthesis
    StmtUnclosedCall = 0x212,
    // E013: Unclosed index bracket
    StmtUnclosedIndex = 0x213,

    // E040: Missing let pattern
    LetMissingPattern = 0x240,
    // E041: Missing let value / assignment RHS
    LetMissingValue = 0x241,
    // E042: Missing let equals sign
    LetMissingEquals = 0x242,
    // E043: Invalid let type or pattern
    LetInvalidTypeOrPattern = 0x243,
    // E044: Invalid provide statement
    ProvideInvalid = 0x244,
    // E045: Invalid defer/errdefer statement
    DeferInvalid = 0x245,
    // E046: Invalid assignment (chained, invalid LHS)
    AssignmentInvalid = 0x246,
    // E047: Invalid compound assignment operator
    CompoundAssignInvalid = 0x247,
    // E048: Invalid expression statement
    ExprStmtInvalid = 0x248,
    // E049: Invalid control flow statement
    ControlFlowInvalid = 0x249,

    // ==========================================================================
    // Meta-System Error Codes (M-prefix)
    // Meta-system grammar violations caught at parser level
    // ==========================================================================

    // M006: Invalid meta stage level (0, negative, or non-integer)
    // Grammar: stage_level = integer_lit (* must be >= 1 *)
    MetaInvalidStage = 0xF006,

    // M205: Duplicate context/using clause (Phase 1 - parser)
    // Grammar: function_def = ... , [ context_clause ] , ... ; (at most one)
    MetaDuplicateUsing = 0xF205,

    // M400: Invalid quote syntax (malformed quote expression)
    // Grammar: quote_expr = 'quote' , [ quote_stage ] , '{' , token_tree , '}' ;
    MetaInvalidQuote = 0xF400,

    // M401: Unquote outside quote (splice operator $ used outside quote block)
    // Grammar: quote_interpolation = splice_operator , ( identifier | '{' , expression , '}' ) ;
    MetaSpliceOutsideQuote = 0xF401,

    // M402: Hygiene violation (accidental variable capture in quote)
    MetaHygieneViolation = 0xF402,

    // M403: Gensym collision (generated name collision in macro expansion)
    MetaGensymCollision = 0xF403,

    // M404: Scope resolution failed (cannot resolve identifier in quote context)
    MetaScopeResolutionFailed = 0xF404,

    // M405: Quote stage error (stage mismatch in quote/unquote nesting)
    MetaQuoteStageError = 0xF405,

    // M406: Lift type mismatch (cannot lift value of this type into generated code)
    MetaLiftTypeMismatch = 0xF406,

    // M407: Invalid token tree (malformed token tree in quote block)
    MetaInvalidTokenTree = 0xF407,

    // M408: Capture not declared (variable captured without explicit capture clause)
    MetaCaptureNotDeclared = 0xF408,

    // M409: Repetition mismatch (mismatched lengths in $[for...] expansion)
    MetaRepetitionMismatch = 0xF409,
}

impl ErrorCode {
    /// Convert error code to its string representation (e.g., "E001", "E0A0")
    pub fn as_str(&self) -> &'static str {
        match self {
            // E001-E009
            ErrorCode::UnterminatedChar => "E001",
            ErrorCode::InvalidEscape => "E002",
            ErrorCode::InvalidNumber => "E003",
            ErrorCode::EmptyChar => "E004",
            ErrorCode::InvalidInterpolation => "E005",
            ErrorCode::UnknownToken => "E006",
            // E010-E019
            ErrorCode::MissingSemicolon => "E010",
            ErrorCode::UnclosedAttribute => "E011",
            ErrorCode::InvalidAttributeArgs => "E012",
            ErrorCode::MissingAttributeName => "E013",
            ErrorCode::InvalidNestedAttribute => "E014",
            ErrorCode::InvalidEmptyCfg => "E015",
            ErrorCode::InvalidEmptyRequires => "E016",
            ErrorCode::InvalidEmptyEnsures => "E017",
            ErrorCode::UnexpectedToken => "E018",
            ErrorCode::MissingBlock => "E019",
            // E020-E029
            ErrorCode::InvalidTheorem => "E020",
            ErrorCode::MissingTheoremName => "E021",
            ErrorCode::InvalidLemma => "E022",
            ErrorCode::UnclosedForall => "E023",
            ErrorCode::UnclosedExists => "E024",
            ErrorCode::InvalidProofKeyword => "E025",
            ErrorCode::InvalidAssert => "E026",
            ErrorCode::InvalidAssume => "E027",
            ErrorCode::MalformedTactic => "E028",
            ErrorCode::ProofNotTerminated => "E029",
            // E030-E039
            ErrorCode::MissingFnName => "E030",
            ErrorCode::MissingFnParams => "E031",
            ErrorCode::MissingFnBody => "E032",
            ErrorCode::InvalidFnVisibility => "E033",
            ErrorCode::DuplicateFnModifier => "E034",
            ErrorCode::InvalidFnParam => "E035",
            ErrorCode::MissingParamType => "E036",
            ErrorCode::InvalidReturnType => "E037",
            ErrorCode::InvalidWhereClauseSyntax => "E038",
            ErrorCode::InvalidUsingClauseSyntax => "E039",
            // E040-E049
            ErrorCode::InvalidThrowsClause => "E040",
            ErrorCode::MissingGenericClose => "E041",
            ErrorCode::EmptyGenericParams => "E042",
            ErrorCode::MissingTypeName => "E043",
            ErrorCode::MissingTypeIs => "E044",
            ErrorCode::MissingTypeBody => "E045",
            ErrorCode::InvalidRecordField => "E046",
            ErrorCode::MissingFieldType => "E047",
            ErrorCode::InvalidVariantSyntax => "E048",
            ErrorCode::DuplicateFieldName => "E049",
            // E050-E059
            ErrorCode::InvalidGenericConstraint => "E050",
            ErrorCode::MissingProtocolBrace => "E051",
            ErrorCode::InvalidProtocolMethod => "E052",
            ErrorCode::InvalidRefinementSyntax => "E053",
            ErrorCode::MissingImplType => "E054",
            ErrorCode::MissingImplFor => "E055",
            ErrorCode::InvalidImplMethod => "E056",
            ErrorCode::MissingImplBrace => "E057",
            ErrorCode::MissingContextName => "E058",
            ErrorCode::MissingContextBody => "E059",
            // E060-E069
            ErrorCode::InvalidContextMethod => "E060",
            ErrorCode::MissingModuleName => "E061",
            ErrorCode::MissingModuleBrace => "E062",
            ErrorCode::InvalidMountSyntax => "E063",
            ErrorCode::InvalidPubUseSyntax => "E064",
            ErrorCode::MissingConstType => "E065",
            ErrorCode::MissingConstValue => "E066",
            ErrorCode::MissingStaticType => "E067",
            ErrorCode::InvalidConstExpr => "E068",
            ErrorCode::DuplicateGenericParam => "E069",
            // E070-E079
            ErrorCode::UnclosedArrayType => "E070",
            ErrorCode::ArrayMissingSize => "E071",
            ErrorCode::ArrayNegativeSize => "E072",
            ErrorCode::ArrayDoubleSemicolon => "E073",
            ErrorCode::ArrayMissingElement => "E074",
            ErrorCode::UnclosedCapability => "E075",
            ErrorCode::EmptyCapability => "E076",
            ErrorCode::CapabilityNoWith => "E077",
            ErrorCode::UnclosedRefinement => "E078",
            ErrorCode::RefinementNoBase => "E079",
            // E080-E089
            ErrorCode::InvalidIntSuffix => "E080",
            ErrorCode::UnclosedConstraintGeneric => "E081",
            ErrorCode::EmptyGenericArgs => "E082",
            ErrorCode::DoubleCommaCapability => "E083",
            ErrorCode::TrailingCommaCapability => "E084",
            ErrorCode::DoubleAngleBracket => "E085",
            ErrorCode::DoubleAmpersandRef => "E086",
            ErrorCode::RefWithoutType => "E087",
            ErrorCode::DoubleCheckedRef => "E088",
            ErrorCode::ConflictingRefModifiers => "E089",
            // E090-E099
            ErrorCode::Rank2MissingParams => "E090",
            ErrorCode::UnclosedFnParams => "E091",
            ErrorCode::FnTypeMissingReturn => "E092",
            ErrorCode::WrongArrowOperator => "E093",
            ErrorCode::UnclosedThrows => "E094",
            ErrorCode::UsingWithoutContext => "E095",
            ErrorCode::AsyncWrongPosition => "E096",
            ErrorCode::UnclosedTupleType => "E097",
            ErrorCode::SingleElementTuple => "E098",
            ErrorCode::UnitWithContent => "E099",
            // E0A0-E0AF
            ErrorCode::ThrowNoExpression => "E0A0",
            ErrorCode::FinallyNoBlock => "E0A1",
            ErrorCode::RecoverMalformedClosure => "E0A2",
            ErrorCode::InvalidAsyncBlock => "E0A3",
            ErrorCode::InvalidAwaitExpr => "E0A4",
            ErrorCode::InvalidSelectArm => "E0A5",
            ErrorCode::InvalidSpawnExpr => "E0A6",
            ErrorCode::MissingChannelOp => "E0A7",
            ErrorCode::UnclosedSelect => "E0A8",
            ErrorCode::InvalidBreak => "E0A9",
            ErrorCode::InvalidContinue => "E0AA",
            ErrorCode::InvalidReturn => "E0AB",
            ErrorCode::InvalidYield => "E0AC",
            // E0B0-E0BF
            ErrorCode::UnclosedGenericArgs => "E0B0",
            ErrorCode::TurbofishMissingType => "E0B1",
            ErrorCode::InvalidTupleIndex => "E0B2",
            ErrorCode::InvalidFieldAccess => "E0B3",
            ErrorCode::InvalidMethodCall => "E0B4",
            ErrorCode::InvalidIndexExpr => "E0B5",
            ErrorCode::InvalidCallArgs => "E0B6",
            ErrorCode::InvalidClosure => "E0B7",
            ErrorCode::InvalidMatch => "E0B8",
            ErrorCode::InvalidIf => "E0B9",
            ErrorCode::InvalidFor => "E0BA",
            ErrorCode::InvalidWhile => "E0BB",
            ErrorCode::InvalidLoop => "E0BC",
            ErrorCode::InvalidRange => "E0BD",
            ErrorCode::InvalidBinaryOp => "E0BE",
            ErrorCode::InvalidUnaryOp => "E0BF",
            // E0C0-E0CF
            ErrorCode::TaggedLiteralMissing => "E0C0",
            ErrorCode::TypeofNoArg => "E0C1",
            ErrorCode::ForallMissingDot => "E0C2",
            ErrorCode::ExistsMissingDot => "E0C3",
            ErrorCode::InvalidComprehension => "E0C4",
            ErrorCode::InvalidPipeline => "E0C5",
            ErrorCode::InvalidTryExpr => "E0C6",
            ErrorCode::InvalidDefer => "E0C7",
            ErrorCode::InvalidProvide => "E0C8",
            ErrorCode::InvalidLetPattern => "E0C9",
            // E0E0-E0EF: Rust syntax migration helpers
            ErrorCode::RustKeywordUsed => "E0E0",
            ErrorCode::RustTypeUsed => "E0E1",
            ErrorCode::RustMacroSyntax => "E0E2",
            // E0D0-E0DF: Grammar violation errors
            ErrorCode::TrailingSeparator => "E0D0",
            ErrorCode::EmptyConstruct => "E0D1",
            ErrorCode::DuplicateClause => "E0D2",
            ErrorCode::InvalidSplice => "E0D3",
            ErrorCode::MissingBlockExpr => "E0D4",
            ErrorCode::EmptyShapeParams => "E0D5",
            // Pattern-specific errors (E070-E089 codes, different hex values)
            ErrorCode::PatternInvalidAt => "E070",
            ErrorCode::PatternInvalidIdentifier => "E071",
            ErrorCode::PatternInvalidRest => "E072",
            ErrorCode::PatternInvalidMut => "E073",
            ErrorCode::PatternEmptyTuple => "E074",
            ErrorCode::PatternInvalidActiveArgs => "E075",
            ErrorCode::PatternInvalidField => "E076",
            ErrorCode::PatternDuplicateField => "E077",
            ErrorCode::PatternNestedOr => "E078",
            ErrorCode::PatternOrBinding => "E079",
            ErrorCode::PatternInvalidType => "E080",
            ErrorCode::PatternInvalidSlice => "E081",
            ErrorCode::PatternInvalidUnicode => "E082",
            ErrorCode::PatternInvalidVariantArgs => "E083",
            ErrorCode::PatternInvalidAnd => "E084",
            ErrorCode::PatternTrailingPipe => "E085",
            ErrorCode::PatternInvalidGuard => "E086",
            ErrorCode::PatternInvalidMatchArm => "E087",
            ErrorCode::PatternInvalidLet => "E088",
            ErrorCode::PatternEmptyOr => "E089",

            // Statement-level errors (VCS specification)
            ErrorCode::StmtUnclosedBlock => "E011",
            ErrorCode::StmtUnclosedCall => "E012",
            ErrorCode::StmtUnclosedIndex => "E013",
            ErrorCode::LetMissingPattern => "E040",
            ErrorCode::LetMissingValue => "E041",
            ErrorCode::LetMissingEquals => "E042",
            ErrorCode::LetInvalidTypeOrPattern => "E043",
            ErrorCode::ProvideInvalid => "E044",
            ErrorCode::DeferInvalid => "E045",
            ErrorCode::AssignmentInvalid => "E046",
            ErrorCode::CompoundAssignInvalid => "E047",
            ErrorCode::ExprStmtInvalid => "E048",
            ErrorCode::ControlFlowInvalid => "E049",

            // Meta-system error codes (M-prefix)
            ErrorCode::MetaInvalidStage => "M006",
            ErrorCode::MetaDuplicateUsing => "M205",
            ErrorCode::MetaInvalidQuote => "M400",
            ErrorCode::MetaSpliceOutsideQuote => "M401",
            ErrorCode::MetaHygieneViolation => "M402",
            ErrorCode::MetaGensymCollision => "M403",
            ErrorCode::MetaScopeResolutionFailed => "M404",
            ErrorCode::MetaQuoteStageError => "M405",
            ErrorCode::MetaLiftTypeMismatch => "M406",
            ErrorCode::MetaInvalidTokenTree => "M407",
            ErrorCode::MetaCaptureNotDeclared => "M408",
            ErrorCode::MetaRepetitionMismatch => "M409",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Result type for parsing operations.
pub type ParseResult<T> = Result<T, List<ParseError>>;

/// A comprehensive parse error with location and context.
#[derive(Clone, PartialEq)]
pub struct ParseError {
    pub kind: ParseErrorKind,
    pub span: Span,
    pub help: Option<Text>,
    /// Error code for categorized diagnostics (e.g., "E001", "E010")
    pub code: Option<Text>,
}

impl fmt::Debug for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Include error code in debug output so tests can match on it
        let code = self.code.as_ref().map(|c| c.as_str()).unwrap_or("E000");
        write!(f, "ParseError({}: {:?} at {:?}", code, self.kind, self.span)?;
        if let Some(help) = &self.help {
            write!(f, ", help: {:?}", help)?;
        }
        write!(f, ")")
    }
}

impl ParseError {
    /// Create a new parse error with the given kind and span.
    /// Uses the default error code from the kind.
    pub fn new(kind: ParseErrorKind, span: Span) -> Self {
        let code = kind.error_code().to_string();
        Self {
            kind,
            span,
            help: None,
            code: Some(code.into()),
        }
    }

    /// Create a new parse error with a specific error code.
    pub fn with_error_code(kind: ParseErrorKind, span: Span, code: ErrorCode) -> Self {
        Self {
            kind,
            span,
            help: None,
            code: Some(Text::from(code.as_str())),
        }
    }

    /// Create a new parse error with a custom error code string.
    pub fn new_with_code(kind: ParseErrorKind, span: Span, code: impl Into<Text>) -> Self {
        Self {
            kind,
            span,
            help: None,
            code: Some(code.into()),
        }
    }

    /// Create an unexpected token error.
    pub fn unexpected(expected: &[TokenKind], found: Token) -> Self {
        let expected_vec: List<TokenKind> = expected.to_vec().into();
        Self::with_error_code(
            ParseErrorKind::UnexpectedToken {
                expected: expected_vec,
                found: found.kind,
            },
            found.span,
            ErrorCode::UnexpectedToken,
        )
    }

    /// Create an unexpected end of file error.
    pub fn unexpected_eof(expected: &[TokenKind], span: Span) -> Self {
        let expected_vec: List<TokenKind> = expected.to_vec().into();
        Self::with_error_code(
            ParseErrorKind::UnexpectedEof {
                expected: expected_vec,
            },
            span,
            ErrorCode::MissingBlock,
        )
    }

    /// Create an invalid syntax error with a custom message.
    pub fn invalid_syntax(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: message.into(),
            },
            span,
            ErrorCode::UnexpectedToken,
        )
    }

    /// Create a mismatched delimiters error.
    pub fn mismatched_delimiters(open: TokenKind, close: TokenKind, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::MismatchedDelimiters { open, close },
            span,
            ErrorCode::UnclosedAttribute,
        )
    }

    /// Create an invalid literal error.
    pub fn invalid_literal(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidLiteral {
                message: message.into(),
            },
            span,
            ErrorCode::InvalidNumber,
        )
    }

    /// Create a duplicate modifier error.
    pub fn duplicate_modifier(modifier: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::DuplicateModifier {
                modifier: modifier.into(),
            },
            span,
            ErrorCode::DuplicateFnModifier,
        )
    }

    /// Create an invalid attribute error.
    pub fn invalid_attribute(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidAttribute {
                message: message.into(),
            },
            span,
            ErrorCode::InvalidAttributeArgs,
        )
    }

    /// Add a helpful message to this error.
    pub fn with_help(mut self, help: impl Into<Text>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Set a specific error code (overrides the default from kind).
    pub fn with_code(mut self, code: ErrorCode) -> Self {
        self.code = Some(Text::from(code.as_str()));
        self
    }

    /// Set a specific error code from a string (overrides the default from kind).
    pub fn with_code_str(mut self, code: impl Into<Text>) -> Self {
        self.code = Some(code.into());
        self
    }

    /// Create a missing semicolon error with context-aware help.
    pub fn missing_semicolon(span: Span, after_what: &str) -> Self {
        Self::with_error_code(ParseErrorKind::MissingSemicolon, span, ErrorCode::MissingSemicolon)
            .with_help(Text::from(format!("add `;` after the {}", after_what)))
    }

    /// Create an unclosed delimiter error with helpful suggestion.
    pub fn unclosed_delimiter(delimiter: char, opening_span: Span, current_span: Span) -> Self {
        let closing = match delimiter {
            '(' => ')',
            '[' => ']',
            '{' => '}',
            '<' => '>',
            _ => delimiter,
        };
        let code = match delimiter {
            '(' => ErrorCode::UnclosedAttribute, // Will be overridden based on context
            '[' => ErrorCode::UnclosedArrayType,
            '{' => ErrorCode::MissingBlock,
            '<' => ErrorCode::UnclosedGenericArgs,
            _ => ErrorCode::UnclosedAttribute,
        };
        Self::with_error_code(ParseErrorKind::UnclosedDelimiter(delimiter), current_span, code)
            .with_help(Text::from(format!(
                "add '{}' to close the '{}' opened at line {}:{}",
                closing, delimiter, opening_span.start, opening_span.end
            )))
    }

    /// Create a helpful error for expected token with suggestions.
    pub fn expected_token(expected: TokenKind, found: Token, suggestion: impl Into<Text>) -> Self {
        Self::unexpected(&[expected], found).with_help(suggestion)
    }

    /// Create an error for invalid type syntax with a suggestion.
    pub fn invalid_type_syntax(
        message: impl Into<Text>,
        span: Span,
        suggestion: impl Into<Text>,
    ) -> Self {
        Self::invalid_syntax(message, span).with_help(suggestion)
    }

    /// Get the error code for this error.
    pub fn error_code(&self) -> &str {
        self.code.as_ref().map(|c| c.as_str()).unwrap_or("E000")
    }

    // ========================================================================
    // Convenience constructors for specific error codes
    // ========================================================================

    // Lexer/literal errors (E001-E009)
    /// E001: Unterminated character literal
    pub fn unterminated_char(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidLiteral { message: "unterminated character literal".into() },
            span,
            ErrorCode::UnterminatedChar,
        )
    }

    /// E001: Unterminated string literal (same error code as unterminated char)
    pub fn unterminated_string(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidLiteral { message: "unterminated string literal".into() },
            span,
            ErrorCode::UnterminatedChar, // E001 covers both unterminated char and string
        )
    }

    /// E002: Invalid escape sequence
    pub fn invalid_escape(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidLiteral { message: message.into() },
            span,
            ErrorCode::InvalidEscape,
        )
    }

    /// E003: Invalid number literal
    pub fn invalid_number(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidLiteral { message: message.into() },
            span,
            ErrorCode::InvalidNumber,
        )
    }

    /// E004: Empty character literal
    pub fn empty_char(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidLiteral { message: "empty character literal".into() },
            span,
            ErrorCode::EmptyChar,
        )
    }

    /// E005: Invalid interpolation syntax
    pub fn invalid_interpolation(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::InvalidInterpolation,
        )
    }

    /// E006: Unknown token
    pub fn unknown_token(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "unknown token".into() },
            span,
            ErrorCode::UnknownToken,
        )
    }

    // Statement/attribute errors (E010-E019)
    /// E011: Unclosed attribute parenthesis
    pub fn unclosed_attribute(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::UnclosedDelimiter('('),
            span,
            ErrorCode::UnclosedAttribute,
        )
    }

    /// E012: Invalid attribute arguments
    pub fn invalid_attribute_args(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidAttribute { message: message.into() },
            span,
            ErrorCode::InvalidAttributeArgs,
        )
    }

    /// E013: Missing attribute name
    pub fn missing_attribute_name(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidAttribute { message: "missing attribute name after @".into() },
            span,
            ErrorCode::MissingAttributeName,
        )
    }

    /// E014: Invalid nested attribute
    pub fn invalid_nested_attribute(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidAttribute { message: "invalid nested attribute".into() },
            span,
            ErrorCode::InvalidNestedAttribute,
        )
    }

    /// E015: Invalid empty cfg attribute
    pub fn invalid_empty_cfg(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidAttribute { message: "empty cfg attribute".into() },
            span,
            ErrorCode::InvalidEmptyCfg,
        )
    }

    /// E016: Invalid empty requires clause
    pub fn invalid_empty_requires(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidAttribute { message: "empty requires clause".into() },
            span,
            ErrorCode::InvalidEmptyRequires,
        )
    }

    /// E017: Invalid empty ensures clause
    pub fn invalid_empty_ensures(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidAttribute { message: "empty ensures clause".into() },
            span,
            ErrorCode::InvalidEmptyEnsures,
        )
    }

    // Proof/theorem errors (E020-E029)
    /// E020: Invalid theorem declaration
    pub fn invalid_theorem(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::InvalidTheorem,
        )
    }

    /// E021: Missing theorem name
    pub fn missing_theorem_name(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "missing theorem name".into() },
            span,
            ErrorCode::MissingTheoremName,
        )
    }

    /// E022: Invalid lemma declaration
    pub fn invalid_lemma(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::InvalidLemma,
        )
    }

    /// E023: Unclosed forall quantifier
    pub fn unclosed_forall(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::UnclosedDelimiter('('),
            span,
            ErrorCode::UnclosedForall,
        )
    }

    /// E024: Unclosed exists quantifier
    pub fn unclosed_exists(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::UnclosedDelimiter('('),
            span,
            ErrorCode::UnclosedExists,
        )
    }

    /// E025: Invalid proof keyword usage
    pub fn invalid_proof_keyword(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::InvalidProofKeyword,
        )
    }

    /// E026: Invalid assert expression
    pub fn invalid_assert(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::InvalidAssert,
        )
    }

    /// E027: Invalid assume expression
    pub fn invalid_assume(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::InvalidAssume,
        )
    }

    /// E028: Malformed tactic declaration
    pub fn malformed_tactic(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::MalformedTactic,
        )
    }

    /// E029: Proof block not properly terminated
    pub fn proof_not_terminated(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::UnclosedDelimiter('{'),
            span,
            ErrorCode::ProofNotTerminated,
        )
    }

    // Function declaration errors (E030-E039)
    /// E030: Missing function name
    pub fn missing_fn_name(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "expected function name after `fn`".into() },
            span,
            ErrorCode::MissingFnName,
        ).with_help("provide a function name: `fn my_function()`")
    }

    /// E031: Missing function parameter list
    pub fn missing_fn_params(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "expected `(` to start parameter list".into() },
            span,
            ErrorCode::MissingFnParams,
        ).with_help("functions require parentheses: `fn name()` or `fn name(x: Int)`")
    }

    /// E032: Missing function body
    pub fn missing_fn_body(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "expected `{` to start function body or `;` for declaration".into() },
            span,
            ErrorCode::MissingFnBody,
        ).with_help("add a body: `fn name() { ... }` or use `;` for protocol method signatures")
    }

    /// E033: Invalid function visibility
    pub fn invalid_fn_visibility(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::InvalidFnVisibility,
        )
    }

    /// E034: Duplicate function modifier
    pub fn duplicate_fn_modifier(modifier: impl Into<Text>, span: Span) -> Self {
        let mod_text: Text = modifier.into();
        Self::with_error_code(
            ParseErrorKind::DuplicateModifier { modifier: mod_text.clone() },
            span,
            ErrorCode::DuplicateFnModifier,
        ).with_help(format!("remove the duplicate `{}` modifier", mod_text))
    }

    /// E035: Invalid function parameter
    pub fn invalid_fn_param(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::InvalidFnParam,
        ).with_help("parameters have the form `name: Type` or `mut name: Type`")
    }

    /// E036: Missing parameter type
    pub fn missing_param_type(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "expected `:` and type after parameter name".into() },
            span,
            ErrorCode::MissingParamType,
        ).with_help("add type annotation: `name: Type`")
    }

    /// E037: Invalid return type syntax
    pub fn invalid_return_type(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::InvalidReturnType,
        ).with_help("return type uses `->`: `fn name() -> ReturnType`")
    }

    /// E038: Invalid where clause syntax
    pub fn invalid_where_clause(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::InvalidWhereClauseSyntax,
        ).with_help("where clause syntax: `where T: Protocol, U: OtherProtocol`")
    }

    /// E039: Invalid using clause syntax
    pub fn invalid_using_clause(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::InvalidUsingClauseSyntax,
        ).with_help("using clause syntax: `using [Context1, Context2]`")
    }

    // Type definition errors (E040-E049)
    /// E040: Invalid throws clause syntax
    pub fn invalid_throws_clause(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::InvalidThrowsClause,
        ).with_help("throws clause syntax: `throws(ErrorType)` or `throws(E1, E2)`")
    }

    /// E041: Missing generic closing bracket
    pub fn missing_generic_close(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::UnclosedDelimiter('<'),
            span,
            ErrorCode::MissingGenericClose,
        ).with_help("add `>` to close the generic parameters")
    }

    /// E042: Empty generic parameters
    pub fn empty_generic_params(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "generic parameters `<>` cannot be empty".into() },
            span,
            ErrorCode::EmptyGenericParams,
        ).with_help("add at least one type parameter: `<T>` or remove the angle brackets")
    }

    /// E069: Duplicate generic parameter name
    pub fn duplicate_generic_param(name: &str, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: format!("duplicate generic parameter `{}`", name).into() },
            span,
            ErrorCode::DuplicateGenericParam,
        ).with_help(format!("each generic parameter must have a unique name; `{}` was already declared", name))
    }

    /// E043: Missing type name
    pub fn missing_type_name(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "expected type name after `type`".into() },
            span,
            ErrorCode::MissingTypeName,
        ).with_help("provide a type name: `type MyType is ...`")
    }

    /// E044: Missing type 'is' keyword
    pub fn missing_type_is(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "expected `is` keyword in type definition".into() },
            span,
            ErrorCode::MissingTypeIs,
        ).with_help("Verum uses `type Name is ...` syntax, not `type Name = ...` or `struct Name`")
    }

    /// E045: Missing type body
    pub fn missing_type_body(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "expected type body after `is`".into() },
            span,
            ErrorCode::MissingTypeBody,
        ).with_help("specify a type body: record `{ fields }`, sum type `A | B`, or protocol `protocol { ... }`")
    }

    /// E046: Invalid record field syntax
    pub fn invalid_record_field(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::InvalidRecordField,
        ).with_help("record fields have the form `name: Type`")
    }

    /// E047: Missing field type
    pub fn missing_field_type(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "expected `:` and type after field name".into() },
            span,
            ErrorCode::MissingFieldType,
        ).with_help("add type annotation: `field_name: FieldType`")
    }

    /// E048: Invalid variant syntax
    pub fn invalid_variant_syntax(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::InvalidVariantSyntax,
        ).with_help("sum type variants: `A | B | C(payload)` or `A | B { fields }`")
    }

    /// E049: Duplicate field name
    pub fn duplicate_field_name(name: impl Into<Text>, span: Span) -> Self {
        let field: Text = name.into();
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: format!("field `{}` is defined multiple times", field).into() },
            span,
            ErrorCode::DuplicateFieldName,
        ).with_help(format!("rename one of the `{}` fields to have a unique name", field))
    }

    // Protocol/implement errors (E050-E059)
    /// E050: Invalid generic constraint
    pub fn invalid_generic_constraint(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::InvalidGenericConstraint,
        ).with_help("constraint syntax: `T: Protocol` or `T: P1 + P2`")
    }

    /// E051: Missing protocol opening brace
    pub fn missing_protocol_brace(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "expected `{` to start protocol body".into() },
            span,
            ErrorCode::MissingProtocolBrace,
        ).with_help("protocol syntax: `type Name is protocol { fn method(); ... }`")
    }

    /// E052: Invalid protocol method
    pub fn invalid_protocol_method(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::InvalidProtocolMethod,
        ).with_help("protocol methods use `fn name(params) -> Type;` (note the semicolon, no body)")
    }

    /// E053: Invalid refinement syntax
    pub fn invalid_refinement_syntax(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::InvalidRefinementSyntax,
        ).with_help("refinement syntax: `{ v: BaseType | predicate(v) }`")
    }

    /// E054: Missing impl type
    pub fn missing_impl_type(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "expected type after `implement`".into() },
            span,
            ErrorCode::MissingImplType,
        ).with_help("syntax: `implement MyType { ... }` or `implement Protocol for MyType { ... }`")
    }

    /// E055: Missing 'for' in trait impl
    pub fn missing_impl_for(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "expected `for` keyword after protocol name".into() },
            span,
            ErrorCode::MissingImplFor,
        ).with_help("protocol implementation syntax: `implement Protocol for TargetType { ... }`")
    }

    /// E056: Invalid impl method
    pub fn invalid_impl_method(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::InvalidImplMethod,
        ).with_help("implement blocks contain function definitions: `fn method() { ... }`")
    }

    /// E057: Missing impl opening brace
    pub fn missing_impl_brace(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "expected `{` to start implement block".into() },
            span,
            ErrorCode::MissingImplBrace,
        ).with_help("implement syntax: `implement Type { fn method() { ... } }`")
    }

    /// E058: Missing context name
    pub fn missing_context_name(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "expected context name after `context`".into() },
            span,
            ErrorCode::MissingContextName,
        ).with_help("context syntax: `context MyContext { ... }`")
    }

    /// E059: Missing context body
    pub fn missing_context_body(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "expected `{` to start context body".into() },
            span,
            ErrorCode::MissingContextBody,
        ).with_help("context syntax: `context Name { type T; fn get() -> T; }`")
    }

    // Context/module/const errors (E060-E069)
    /// E060: Invalid context method
    pub fn invalid_context_method(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::InvalidContextMethod,
        )
    }

    /// E061: Missing module name
    pub fn missing_module_name(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "expected module name after `mod`".into() },
            span,
            ErrorCode::MissingModuleName,
        )
    }

    /// E062: Missing module opening brace
    pub fn missing_module_brace(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "missing opening brace in module declaration".into() },
            span,
            ErrorCode::MissingModuleBrace,
        )
    }

    /// E063: Invalid mount syntax
    pub fn invalid_mount_syntax(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::InvalidMountSyntax,
        )
    }

    /// E064: Invalid pub use syntax
    pub fn invalid_pub_use_syntax(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::InvalidPubUseSyntax,
        )
    }

    /// E065: Missing const type
    pub fn missing_const_type(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "missing type annotation in const declaration".into() },
            span,
            ErrorCode::MissingConstType,
        )
    }

    /// E066: Missing const value
    pub fn missing_const_value(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "missing value in const declaration".into() },
            span,
            ErrorCode::MissingConstValue,
        )
    }

    /// E067: Missing static type
    pub fn missing_static_type(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "missing type annotation in static declaration".into() },
            span,
            ErrorCode::MissingStaticType,
        )
    }

    // Array/capability type errors (E070-E079)
    /// E070: Unclosed array type
    pub fn unclosed_array_type(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::UnclosedDelimiter('['),
            span,
            ErrorCode::UnclosedArrayType,
        ).with_help("add `]` to close the array type: `[T; N]`")
    }

    /// E071: Array type missing size
    pub fn array_missing_size(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "expected array size after `;`".into() },
            span,
            ErrorCode::ArrayMissingSize,
        ).with_help("array syntax: `[ElementType; size]`, e.g., `[Int; 10]`")
    }

    /// E072: Array type with negative size
    pub fn array_negative_size(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "array size must be a non-negative integer".into() },
            span,
            ErrorCode::ArrayNegativeSize,
        ).with_help("use a non-negative size: `[T; 0]` or `[T; 10]`")
    }

    /// E073: Array type with double semicolon
    pub fn array_double_semicolon(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "found `;;` but expected single `;` in array type".into() },
            span,
            ErrorCode::ArrayDoubleSemicolon,
        ).with_help("use single semicolon: `[T; N]`, not `[T;; N]`")
    }

    /// E074: Array missing element type
    pub fn array_missing_element(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "expected element type in array".into() },
            span,
            ErrorCode::ArrayMissingElement,
        ).with_help("specify element type: `[Int; 10]` or `[MyType; N]`")
    }

    /// E075: Unclosed capability list
    pub fn unclosed_capability(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::UnclosedDelimiter(']'),
            span,
            ErrorCode::UnclosedCapability,
        ).with_help("add `]` to close the capability list")
    }

    /// E076: Empty capability list
    pub fn empty_capability(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "capability list `[]` cannot be empty".into() },
            span,
            ErrorCode::EmptyCapability,
        ).with_help("add at least one capability or remove the brackets")
    }

    /// E077: Capability syntax without 'with' keyword
    pub fn capability_no_with(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "capability type requires `with` keyword".into() },
            span,
            ErrorCode::CapabilityNoWith,
        ).with_help("syntax: `Type with [Cap1, Cap2]`")
    }

    /// E078: Unclosed refinement type
    pub fn unclosed_refinement(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::UnclosedDelimiter('{'),
            span,
            ErrorCode::UnclosedRefinement,
        ).with_help("add `}` to close the refinement type")
    }

    /// E079: Refinement without base type
    pub fn refinement_no_base(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "refinement type requires a base type".into() },
            span,
            ErrorCode::RefinementNoBase,
        ).with_help("syntax: `{ v: BaseType | predicate(v) }`")
    }

    // Reference/pointer type errors (E080-E089)
    /// E080: Invalid integer type suffix
    pub fn invalid_int_suffix(suffix: impl Into<Text>, span: Span) -> Self {
        let suf: Text = suffix.into();
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: format!("invalid integer type suffix `{}`", suf).into() },
            span,
            ErrorCode::InvalidIntSuffix,
        ).with_help("valid suffixes: i8, i16, i32, i64, i128, u8, u16, u32, u64, u128")
    }

    /// E081: Unclosed type constraint generic
    pub fn unclosed_constraint_generic(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::UnclosedDelimiter('<'),
            span,
            ErrorCode::UnclosedConstraintGeneric,
        ).with_help("add `>` to close the generic constraint")
    }

    /// E082: Empty generic type arguments
    pub fn empty_generic_args(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "generic type arguments `<>` cannot be empty".into() },
            span,
            ErrorCode::EmptyGenericArgs,
        ).with_help("provide at least one type argument: `Type<T>` or remove the angle brackets")
    }

    /// E083: Double comma in capability list
    pub fn double_comma_capability(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "found `,,` but expected single comma between capabilities".into() },
            span,
            ErrorCode::DoubleCommaCapability,
        ).with_help("use single commas: `[Cap1, Cap2]`")
    }

    /// E084: Trailing comma in capability list
    pub fn trailing_comma_capability(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "trailing comma in capability list".into() },
            span,
            ErrorCode::TrailingCommaCapability,
        ).with_help("remove the trailing comma or add another capability")
    }

    /// E085: Double opening angle bracket
    pub fn double_angle_bracket(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "found `<<` but expected single `<` for generics".into() },
            span,
            ErrorCode::DoubleAngleBracket,
        ).with_help("use single angle bracket: `Type<T>`, not `Type<<T>>`; `<<` is a bit-shift operator")
    }

    /// E086: Invalid double ampersand in reference
    pub fn double_ampersand_ref(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "double ampersand `&&` in reference type".into() },
            span,
            ErrorCode::DoubleAmpersandRef,
        )
    }

    /// E087: Reference without type
    pub fn ref_without_type(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "reference `&` without type".into() },
            span,
            ErrorCode::RefWithoutType,
        )
    }

    /// E088: Double checked in reference
    pub fn double_checked_ref(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "duplicate reference modifier (e.g., `checked checked`)".into() },
            span,
            ErrorCode::DoubleCheckedRef,
        ).with_help("use a single modifier: `&checked T`, `&mut T`, or `&unsafe T`")
    }

    /// E089: Conflicting reference modifiers
    pub fn conflicting_ref_modifiers(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "conflicting reference modifiers (e.g., `checked unsafe`)".into() },
            span,
            ErrorCode::ConflictingRefModifiers,
        ).with_help("reference modifiers are mutually exclusive: choose one of `checked`, `unsafe`, or neither")
    }

    // Exception/control flow errors (E0A0-E0AF)
    /// E0A0: Throw without expression
    pub fn throw_no_expression(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "`throw` requires an expression to throw".into() },
            span,
            ErrorCode::ThrowNoExpression,
        ).with_help("syntax: `throw error_value` or `throw MyError(\"message\")`")
    }

    /// E0A8: Unclosed select
    pub fn unclosed_select(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::UnclosedDelimiter('{'),
            span,
            ErrorCode::UnclosedSelect,
        ).with_help("add `}` to close the select expression")
    }

    // Expression syntax errors (E0B0-E0BF)
    /// E0B0: Unclosed generic args
    pub fn unclosed_generic_args(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::UnclosedDelimiter('<'),
            span,
            ErrorCode::UnclosedGenericArgs,
        ).with_help("add `>` to close the generic type arguments")
    }

    /// E0B7: Invalid closure
    pub fn invalid_closure(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::InvalidClosure,
        ).with_help("closure syntax: `|params| body` or `|x: Int| -> Int { x + 1 }`")
    }

    /// E0B8: Invalid match expression
    pub fn invalid_match(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::InvalidMatch,
        ).with_help("match syntax: `match expr { pattern => result, ... }`")
    }

    /// E0C9: Invalid let pattern
    pub fn invalid_let_pattern(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::InvalidLetPattern,
        ).with_help("let syntax: `let pattern = value` or `let name: Type = value`")
    }

    // Function type errors (E090-E099)
    /// E090: Rank-2 function missing parameter list
    /// Rank-2 polymorphism: `fn<T>(...)` where the function works for ANY T chosen by callee.
    pub fn rank2_missing_params(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: "rank-2 function type `fn<T>` requires a parameter list after the generic parameters".into()
            },
            span,
            ErrorCode::Rank2MissingParams,
        ).with_help("rank-2 functions have the form `fn<T>(params) -> ReturnType` where T is universally quantified")
    }

    /// E091: Unclosed function parameter list
    pub fn unclosed_fn_params(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::UnclosedDelimiter('('),
            span,
            ErrorCode::UnclosedFnParams,
        ).with_help("add `)` to close the parameter list")
    }

    /// E092: Function type missing return type
    pub fn fn_type_missing_return(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "function type missing return type after `->` arrow".into() },
            span,
            ErrorCode::FnTypeMissingReturn,
        ).with_help("specify the return type, e.g., `fn(Int) -> Int` or use `()` for no return value")
    }

    /// E093: Wrong arrow operator (=> instead of ->)
    pub fn wrong_arrow_operator(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "found `=>` but expected `->` for function return type".into() },
            span,
            ErrorCode::WrongArrowOperator,
        ).with_help("use `->` for function return types: `fn(x: Int) -> Int`; `=>` is used for match arms")
    }

    /// E094: Unclosed throws clause in function type
    pub fn unclosed_throws(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::UnclosedDelimiter('('),
            span,
            ErrorCode::UnclosedThrows,
        ).with_help("close the throws clause with `)`, e.g., `throws(Error1, Error2)`")
    }

    /// E095: Using clause without context list
    pub fn using_without_context(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "`using` keyword requires a context list".into() },
            span,
            ErrorCode::UsingWithoutContext,
        ).with_help("specify contexts in brackets: `using [Database, Logger]`")
    }

    /// E096: Async keyword in wrong position
    pub fn async_wrong_position(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "`async` modifier appears in an invalid position".into() },
            span,
            ErrorCode::AsyncWrongPosition,
        ).with_help("`async` should come before `fn`: `async fn name() -> T`")
    }

    /// E097: Unclosed tuple type
    pub fn unclosed_tuple_type(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::UnclosedDelimiter('('),
            span,
            ErrorCode::UnclosedTupleType,
        )
    }

    /// E098: Single element tuple invalid
    pub fn single_element_tuple(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "single element tuple is not valid, use parentheses or add trailing comma".into() },
            span,
            ErrorCode::SingleElementTuple,
        )
    }

    /// E099: Unit type with content
    pub fn unit_with_content(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "unit type `()` cannot contain content".into() },
            span,
            ErrorCode::UnitWithContent,
        )
    }

    // Additional errors used by type parser (aliased from new codes)
    /// Variant pipe errors (for union types)
    pub fn empty_variant_pipe(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "empty variant between `|` pipes in sum type".into() },
            span,
            ErrorCode::InvalidVariantSyntax,  // E048
        ).with_help("each `|` must separate variant names: `type T is A | B | C`")
    }

    pub fn trailing_pipe_union(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "trailing `|` in sum type without a following variant".into() },
            span,
            ErrorCode::InvalidVariantSyntax,  // E048
        ).with_help("remove the trailing `|` or add another variant")
    }

    pub fn leading_pipe_no_context(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "unexpected leading `|` in type expression".into() },
            span,
            ErrorCode::InvalidVariantSyntax,  // E048
        ).with_help("leading `|` is only allowed in `type` definitions, not in type expressions")
    }

    /// Constraint/bound errors (reuse function type errors)
    pub fn missing_constraint(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "expected a type constraint after `:`".into() },
            span,
            ErrorCode::InvalidGenericConstraint,  // E050
        ).with_help("specify a protocol bound: `T: Protocol` or multiple bounds: `T: Proto1 + Proto2`")
    }

    pub fn double_colon_constraint(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "found `::` but expected `:` for type constraint".into() },
            span,
            ErrorCode::InvalidGenericConstraint,  // E050
        ).with_help("use single colon for constraints: `T: Protocol`; `::` is for path access")
    }

    pub fn double_negation_bound(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "double negation `!!` is not valid in type bounds".into() },
            span,
            ErrorCode::InvalidGenericConstraint,  // E050
        ).with_help("use single negation for negative bounds: `T: !Send`")
    }

    pub fn dyn_no_protocol(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "`dyn` requires at least one protocol bound".into() },
            span,
            ErrorCode::InvalidProtocolMethod,  // E052
        )
    }

    pub fn existential_no_bounds(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "existential type requires bounds after colon".into() },
            span,
            ErrorCode::InvalidGenericConstraint,  // E050
        )
    }

    pub fn existential_missing_colon(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "existential type requires colon between type parameter and bounds".into() },
            span,
            ErrorCode::InvalidGenericConstraint,  // E050
        )
    }

    // =========================================================================
    // Pattern errors (E070-E089)
    // =========================================================================

    /// E070: Invalid @ binding pattern (missing identifier before @, etc.)
    pub fn pattern_invalid_at(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::PatternInvalidAt,
        )
    }

    /// E071: Invalid identifier in pattern
    pub fn pattern_invalid_identifier(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::PatternInvalidIdentifier,
        )
    }

    /// E072: Invalid rest/spread pattern position
    pub fn pattern_invalid_rest(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::PatternInvalidRest,
        )
    }

    /// E073: Invalid mut pattern
    pub fn pattern_invalid_mut(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::PatternInvalidMut,
        )
    }

    /// E074: Empty tuple pattern with trailing comma
    pub fn pattern_empty_tuple(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "empty tuple pattern is not valid".into() },
            span,
            ErrorCode::PatternEmptyTuple,
        )
    }

    /// E074: Unclosed active pattern (missing closing parenthesis)
    pub fn pattern_unclosed_active(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::PatternEmptyTuple, // Reuses E074 for unclosed active patterns
        )
    }

    /// E075: Invalid active pattern arguments
    pub fn pattern_invalid_active_args(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::PatternInvalidActiveArgs,
        )
    }

    /// E076: Invalid field pattern syntax
    pub fn pattern_invalid_field(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::PatternInvalidField,
        )
    }

    /// E077: Duplicate field in pattern
    pub fn pattern_duplicate_field(field_name: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: Text::from(format!("duplicate field '{}' in pattern", field_name.into())),
            },
            span,
            ErrorCode::PatternDuplicateField,
        )
    }

    /// E078: Nested or-pattern without parentheses
    pub fn pattern_nested_or(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "nested or-pattern requires parentheses".into() },
            span,
            ErrorCode::PatternNestedOr,
        )
    }

    /// E078: Rest pattern in invalid position
    pub fn pattern_rest_position(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::PatternNestedOr, // Reuses E078 for rest position errors
        )
    }

    /// E079: Or-pattern with inconsistent bindings
    pub fn pattern_or_binding(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::PatternOrBinding,
        )
    }

    /// E080: Invalid pattern type annotation
    pub fn pattern_invalid_type(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::PatternInvalidType,
        )
    }

    /// E081: Invalid slice pattern syntax
    pub fn pattern_invalid_slice(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::PatternInvalidSlice,
        )
    }

    /// E082: Invalid unicode pattern
    pub fn pattern_invalid_unicode(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::PatternInvalidUnicode,
        )
    }

    /// E083: Invalid variant pattern arguments
    pub fn pattern_invalid_variant_args(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::PatternInvalidVariantArgs,
        )
    }

    /// E084: Invalid and-pattern (combination)
    pub fn pattern_invalid_and(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::PatternInvalidAnd,
        )
    }

    /// E085: Trailing pipe in pattern (also used for missing guard expression)
    pub fn pattern_trailing_pipe(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "trailing pipe in pattern".into() },
            span,
            ErrorCode::PatternTrailingPipe,
        )
    }

    /// E085: Missing guard expression (if/where without expression)
    pub fn pattern_missing_guard(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::PatternTrailingPipe,
        )
    }

    /// E086: Invalid guard expression
    pub fn pattern_invalid_guard(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::PatternInvalidGuard,
        )
    }

    /// E086: Invalid slice pattern syntax (double rest, triple dots, etc.)
    pub fn pattern_invalid_slice_syntax(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::PatternInvalidGuard, // Reuses E086
        )
    }

    /// E087: Invalid match arm syntax
    pub fn pattern_invalid_match_arm(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::PatternInvalidMatchArm,
        )
    }

    /// E088: Invalid let pattern
    pub fn pattern_invalid_let(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::PatternInvalidLet,
        )
    }

    /// E089: Empty or-pattern (consecutive pipes)
    pub fn pattern_empty_or(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "empty or-pattern (consecutive pipes)".into() },
            span,
            ErrorCode::PatternEmptyOr,
        )
    }

    // =========================================================================
    // Statement errors (E011-E013, E040-E049) - VCS specification
    // =========================================================================

    /// E011: Unclosed block statement
    pub fn stmt_unclosed_block(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::UnclosedDelimiter('{'),
            span,
            ErrorCode::StmtUnclosedBlock,
        )
    }

    /// E012: Unclosed function call parenthesis
    pub fn stmt_unclosed_call(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::UnclosedDelimiter('('),
            span,
            ErrorCode::StmtUnclosedCall,
        )
    }

    /// E013: Unclosed index bracket
    pub fn stmt_unclosed_index(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::UnclosedDelimiter('['),
            span,
            ErrorCode::StmtUnclosedIndex,
        )
    }

    /// E040: Missing let pattern
    pub fn let_missing_pattern(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "missing pattern in let statement".into() },
            span,
            ErrorCode::LetMissingPattern,
        )
    }

    /// E041: Missing let value / assignment RHS
    pub fn let_missing_value(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "missing value after '=' in let statement".into() },
            span,
            ErrorCode::LetMissingValue,
        )
    }

    /// E042: Missing let equals sign
    pub fn let_missing_equals(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: "missing '=' in let statement".into() },
            span,
            ErrorCode::LetMissingEquals,
        )
    }

    /// E043: Invalid let type or pattern
    pub fn let_invalid_type_or_pattern(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::LetInvalidTypeOrPattern,
        )
    }

    /// E044: Invalid provide statement
    pub fn provide_invalid(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::ProvideInvalid,
        )
    }

    /// E045: Invalid defer/errdefer statement
    pub fn defer_invalid(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::DeferInvalid,
        )
    }

    /// E046: Invalid assignment (chained, invalid LHS)
    pub fn assignment_invalid(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::AssignmentInvalid,
        )
    }

    /// E047: Invalid compound assignment operator
    pub fn compound_assign_invalid(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::CompoundAssignInvalid,
        )
    }

    /// E048: Invalid expression statement
    pub fn expr_stmt_invalid(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::ExprStmtInvalid,
        )
    }

    /// E049: Invalid control flow statement
    pub fn control_flow_invalid(message: impl Into<Text>, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax { message: message.into() },
            span,
            ErrorCode::ControlFlowInvalid,
        )
    }

    // =========================================================================
    // Grammar violation errors (E0D0-E0DF) - Strict EBNF enforcement
    // =========================================================================

    /// E0D0: Trailing separator without following element
    /// Used for trailing `+` in type bounds, trailing `|` in variants, etc.
    pub fn trailing_separator(separator: &str, construct: &str, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: Text::from(format!(
                    "trailing '{}' in {} - expected another element after separator",
                    separator, construct
                )),
            },
            span,
            ErrorCode::TrailingSeparator,
        )
    }

    /// E0D1: Empty construct where grammar requires content
    /// Used for empty refinement `{}`, empty blocks, etc.
    pub fn empty_construct(construct: &str, expected: &str, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: Text::from(format!(
                    "empty {} - {}",
                    construct, expected
                )),
            },
            span,
            ErrorCode::EmptyConstruct,
        )
    }

    /// E0D2: Duplicate clause (e.g., multiple where clauses)
    pub fn duplicate_clause(clause: &str, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: Text::from(format!(
                    "duplicate '{}' clause - only one {} clause is allowed",
                    clause, clause
                )),
            },
            span,
            ErrorCode::DuplicateClause,
        )
    }

    /// E0D3: Invalid splice ($ without identifier or expression)
    pub fn invalid_splice_syntax(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: "invalid splice - expected identifier or '{expr}' after '$'".into(),
            },
            span,
            ErrorCode::InvalidSplice,
        )
    }

    /// E0D4: Missing required block expression
    /// Used for nursery without block, meta without block, etc.
    pub fn missing_block_expr(construct: &str, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: Text::from(format!(
                    "{} requires a block expression",
                    construct
                )),
            },
            span,
            ErrorCode::MissingBlockExpr,
        )
    }

    /// E0D5: Empty shape parameters in tensor
    pub fn empty_shape_params(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: "tensor shape parameters cannot be empty - expected at least one dimension".into(),
            },
            span,
            ErrorCode::EmptyShapeParams,
        )
    }

    // =========================================================================
    // Rust Syntax Migration Helpers (E0E0-E0E2)
    // =========================================================================

    /// E0E0: Rust keyword used instead of Verum equivalent
    pub fn rust_keyword_used(
        rust_keyword: &str,
        verum_equivalent: &str,
        span: Span,
    ) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: Text::from(format!(
                    "'{}' is not a Verum keyword",
                    rust_keyword
                )),
            },
            span,
            ErrorCode::RustKeywordUsed,
        )
        .with_help(Text::from(format!(
            "did you mean `{}`? Verum uses '{}' instead of Rust's '{}'",
            verum_equivalent, verum_equivalent, rust_keyword
        )))
    }

    /// E0E1: Rust type name used instead of Verum semantic type
    pub fn rust_type_used(
        rust_type: &str,
        verum_type: &str,
        span: Span,
    ) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: Text::from(format!(
                    "'{}' is a Rust type, not a Verum type",
                    rust_type
                )),
            },
            span,
            ErrorCode::RustTypeUsed,
        )
        .with_help(Text::from(format!(
            "did you mean `{}`? Verum uses semantic type names: {} -> {}",
            verum_type, rust_type, verum_type
        )))
    }

    /// E0E2: Rust macro syntax used instead of Verum syntax (3-arg version)
    pub fn rust_macro_syntax_with_equivalent(
        rust_macro: &str,
        verum_equivalent: &str,
        span: Span,
    ) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: Text::from(format!(
                    "'{}!' is Rust macro syntax, not valid in Verum",
                    rust_macro
                )),
            },
            span,
            ErrorCode::RustMacroSyntax,
        )
        .with_help(Text::from(format!(
            "did you mean `{}`? Verum does not use '!' for macros or built-in functions",
            verum_equivalent
        )))
    }

    // =========================================================================
    // Meta-System Errors (M-prefix)
    // Meta-system grammar violations caught at parser level
    // =========================================================================

    /// M006: Invalid meta stage level (0, negative, float, or non-integer)
    /// Grammar: stage_level = integer_lit (* must be >= 1 *)
    pub fn meta_invalid_stage(span: Span, reason: &str) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: Text::from(format!("invalid meta stage level: {}", reason)),
            },
            span,
            ErrorCode::MetaInvalidStage,
        )
    }

    /// M401: Empty quote block (invalid quote syntax)
    /// Grammar: quote_expr = 'quote' , [ quote_stage ] , '{' , token_tree , '}' ;
    pub fn meta_empty_quote(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: "empty quote block - quote blocks must contain at least one item".into(),
            },
            span,
            ErrorCode::MetaInvalidQuote,
        )
    }

    /// M205: Duplicate using clause
    /// Grammar: function_def = ... , [ context_clause ] , ... ; (at most one)
    pub fn meta_duplicate_using(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: "duplicate using clause - functions can have at most one using clause".into(),
            },
            span,
            ErrorCode::MetaDuplicateUsing,
        )
    }

    /// E038: Where clause order error (meta where must come before generic where)
    /// Grammar: function_def = ... , [ generic_where_clause ] , [ meta_where_clause ] , ... ;
    /// Note: No M-prefix code defined in spec for this parser-level error
    pub fn where_clause_order(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: "wrong where clause order - meta where clause must come before generic where clause".into(),
            },
            span,
            ErrorCode::InvalidWhereClauseSyntax,
        )
    }

    /// M401: Invalid quote syntax
    /// Grammar: quote_expr syntax errors
    pub fn meta_invalid_quote(span: Span, reason: &str) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: Text::from(format!("invalid quote syntax: {}", reason)),
            },
            span,
            ErrorCode::MetaInvalidQuote,
        )
    }

    /// M402: Splice used outside quote block
    /// Grammar: quote_interpolation = '$' , ( identifier | '{' , expression , '}' ) ;
    pub fn meta_splice_outside_quote(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: "splice operator '$' can only be used inside quote blocks".into(),
            },
            span,
            ErrorCode::MetaSpliceOutsideQuote,
        )
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Use the global source file registry to convert span to human-readable location
        let line_col = global_span_to_line_col(self.span);
        write!(
            f,
            "{} at {}:{}:{}",
            self.kind, line_col.file, line_col.line, line_col.column
        )?;

        if let Some(help) = &self.help {
            write!(f, "\n  help: {}", help)?;
        }

        Ok(())
    }
}

impl std::error::Error for ParseError {}

/// The kinds of parse errors that can occur.
#[derive(Debug, Clone, PartialEq)]
pub enum ParseErrorKind {
    /// An unexpected token was encountered.
    UnexpectedToken {
        expected: List<TokenKind>,
        found: TokenKind,
    },
    /// Unexpected end of file.
    UnexpectedEof { expected: List<TokenKind> },
    /// Invalid syntax with a custom message.
    InvalidSyntax { message: Text },
    /// Mismatched delimiters (e.g., opening `{` but closing `)`).
    MismatchedDelimiters { open: TokenKind, close: TokenKind },
    /// Invalid literal (e.g., malformed number or string).
    InvalidLiteral { message: Text },
    /// Duplicate modifier (e.g., `pub pub fn foo()`).
    DuplicateModifier { modifier: Text },
    /// Invalid attribute.
    InvalidAttribute { message: Text },
    /// Unclosed delimiter - opening delimiter not closed.
    UnclosedDelimiter(char),
    /// Missing closing delimiter - expected delimiter to close a block.
    MissingClosingDelimiter(char),
    /// Missing semicolon after statement.
    MissingSemicolon,
}

impl ParseErrorKind {
    /// Get the standard error code for this error kind.
    /// Returns a default code based on the error variant.
    /// More specific codes should be set via ParseError::with_code().
    pub fn error_code(&self) -> &'static str {
        match self {
            ParseErrorKind::UnexpectedToken { .. } => ErrorCode::UnexpectedToken.as_str(),
            ParseErrorKind::UnexpectedEof { .. } => ErrorCode::MissingBlock.as_str(),
            ParseErrorKind::InvalidSyntax { .. } => ErrorCode::UnexpectedToken.as_str(),
            ParseErrorKind::MismatchedDelimiters { .. } => ErrorCode::UnclosedAttribute.as_str(),
            ParseErrorKind::InvalidLiteral { .. } => ErrorCode::InvalidNumber.as_str(),
            ParseErrorKind::DuplicateModifier { .. } => ErrorCode::DuplicateFnModifier.as_str(),
            ParseErrorKind::InvalidAttribute { .. } => ErrorCode::InvalidAttributeArgs.as_str(),
            ParseErrorKind::UnclosedDelimiter(ch) => match ch {
                '(' => ErrorCode::UnclosedAttribute.as_str(),
                '[' => ErrorCode::UnclosedArrayType.as_str(),
                '{' => ErrorCode::MissingBlock.as_str(),
                '<' => ErrorCode::UnclosedGenericArgs.as_str(),
                _ => ErrorCode::UnclosedAttribute.as_str(),
            },
            ParseErrorKind::MissingClosingDelimiter(ch) => match ch {
                '(' => ErrorCode::UnclosedFnParams.as_str(),
                '[' => ErrorCode::UnclosedArrayType.as_str(),
                '{' => ErrorCode::MissingModuleBrace.as_str(),
                '<' => ErrorCode::UnclosedConstraintGeneric.as_str(),
                _ => ErrorCode::UnclosedAttribute.as_str(),
            },
            ParseErrorKind::MissingSemicolon => ErrorCode::MissingSemicolon.as_str(),
        }
    }
}

impl fmt::Display for ParseErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseErrorKind::UnexpectedToken { expected, found } => {
                write!(f, "unexpected {}", format_token_kind(found))?;
                if !expected.is_empty() {
                    write!(f, ", expected ")?;
                    write_expected_tokens(f, expected)?;
                }
                Ok(())
            }
            ParseErrorKind::UnexpectedEof { expected } => {
                write!(f, "unexpected end of file")?;
                if !expected.is_empty() {
                    write!(f, "; expected ")?;
                    write_expected_tokens(f, expected)?;
                }
                Ok(())
            }
            ParseErrorKind::InvalidSyntax { message } => {
                write!(f, "{}", message)
            }
            ParseErrorKind::MismatchedDelimiters { open, close } => {
                write!(
                    f,
                    "mismatched delimiters: expected {} to match opening {}",
                    format_token_kind(close),
                    format_token_kind(open)
                )
            }
            ParseErrorKind::InvalidLiteral { message } => {
                write!(f, "invalid literal: {}", message)
            }
            ParseErrorKind::DuplicateModifier { modifier } => {
                write!(f, "duplicate modifier '{}'", modifier)
            }
            ParseErrorKind::InvalidAttribute { message } => {
                write!(f, "invalid attribute: {}", message)
            }
            ParseErrorKind::UnclosedDelimiter(ch) => {
                write!(
                    f,
                    "unclosed delimiter '{}' - did you forget to close it?",
                    ch
                )
            }
            ParseErrorKind::MissingClosingDelimiter(ch) => {
                write!(
                    f,
                    "missing closing delimiter '{}' - expected to close this block",
                    ch
                )
            }
            ParseErrorKind::MissingSemicolon => {
                write!(f, "missing semicolon ';' after statement")
            }
        }
    }
}

/// Format a token kind for display in error messages.
fn format_token_kind(kind: &TokenKind) -> String {
    kind.description().to_string()
}

/// Write a list of expected tokens in a human-readable format.
fn write_expected_tokens(f: &mut fmt::Formatter<'_>, expected: &[TokenKind]) -> fmt::Result {
    match expected.len() {
        0 => Ok(()),
        1 => write!(f, "{}", format_token_kind(&expected[0])),
        2 => write!(
            f,
            "{} or {}",
            format_token_kind(&expected[0]),
            format_token_kind(&expected[1])
        ),
        _ => {
            for (i, kind) in expected.iter().enumerate() {
                if i > 0 {
                    if i == expected.len() - 1 {
                        write!(f, ", or ")?;
                    } else {
                        write!(f, ", ")?;
                    }
                }
                write!(f, "{}", format_token_kind(kind))?;
            }
            Ok(())
        }
    }
}

// Convert ParseError to VerumError for compatibility with the broader error system
impl From<ParseError> for VerumError {
    fn from(err: ParseError) -> Self {
        let message = format!("{}", err.kind);
        // Use the global source file registry to convert span to human-readable location
        let line_col = global_span_to_line_col(err.span);
        let mut verum_err = Self::parse(message).at_location(
            line_col.file.clone(),
            line_col.line as u32,
            line_col.column as u32,
        );

        if let Some(help) = err.help {
            verum_err = verum_err.with_context(help.to_string());
        }

        verum_err
    }
}

// ============================================================================
// Rust Migration Hints - Helpers for developers transitioning from Rust
// ============================================================================

impl ParseError {
    /// Create an error when user tries to use Rust's `struct` keyword
    pub fn rust_struct_syntax(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: "Verum uses `type Name is { ... }` instead of `struct Name { ... }`".into()
            },
            span,
            ErrorCode::UnexpectedToken,
        ).with_help("replace `struct Point { x: f64, y: f64 }` with `type Point is { x: Float, y: Float }`")
    }

    /// Create an error when user tries to use Rust's `enum` keyword
    pub fn rust_enum_syntax(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: "Verum uses `type Name is A | B` instead of `enum Name { A, B }`".into()
            },
            span,
            ErrorCode::UnexpectedToken,
        ).with_help("replace `enum Option { None, Some(T) }` with `type Option<T> is None | Some(T)`")
    }

    /// Create an error when user tries to use Rust's `trait` keyword
    pub fn rust_trait_syntax(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: "Verum uses `type Name is protocol { ... }` instead of `trait Name { ... }`".into()
            },
            span,
            ErrorCode::UnexpectedToken,
        ).with_help("replace `trait Iterator { ... }` with `type Iterator is protocol { ... }`")
    }

    /// Create an error when user tries to use Rust's `impl` keyword
    pub fn rust_impl_syntax(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: "Verum uses `implement` instead of `impl`".into()
            },
            span,
            ErrorCode::UnexpectedToken,
        ).with_help("replace `impl MyType { ... }` with `implement MyType { ... }`")
    }

    /// Create an error when user tries to use Rust's `use` keyword for imports
    pub fn rust_use_syntax(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: "Verum uses `mount` instead of `use` for imports".into()
            },
            span,
            ErrorCode::UnexpectedToken,
        ).with_help("replace `use foo::bar` with `mount foo.bar`")
    }

    /// Create an error when user tries to use Rust's `crate` keyword
    pub fn rust_crate_syntax(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: "Verum uses `cog` instead of `crate`".into()
            },
            span,
            ErrorCode::UnexpectedToken,
        ).with_help("replace `crate::module` with `cog.module`")
    }

    /// Create an error when user tries to use Rust's `::` path separator
    pub fn rust_path_separator(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: "Verum uses `.` instead of `::` for path access".into()
            },
            span,
            ErrorCode::UnexpectedToken,
        ).with_help("replace `std::io::Read` with `std.io.Read`")
    }

    /// Create an error when user tries to use Rust macro syntax with `!`
    pub fn rust_macro_syntax(name: &str, span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: format!("Verum doesn't use `!` for macros - `{}!` is invalid", name).into()
            },
            span,
            ErrorCode::UnexpectedToken,
        ).with_help(format!(
            "built-in functions don't need `!`: use `{}()` instead of `{}!()`; for macros use `@{}(...)`",
            name, name, name
        ))
    }

    /// Create an error when user tries to use Rust's `#[...]` attribute syntax
    pub fn rust_attribute_syntax(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: "Verum uses `@attribute(...)` instead of `#[attribute(...)]`".into()
            },
            span,
            ErrorCode::UnexpectedToken,
        ).with_help("replace `#[derive(Debug)]` with `@derive(Debug)`")
    }

    /// Create an error when user tries to use Rust's `Vec<T>` type
    pub fn rust_vec_type(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: "Verum uses `List<T>` instead of `Vec<T>`".into()
            },
            span,
            ErrorCode::UnexpectedToken,
        ).with_help("replace `Vec<Int>` with `List<Int>`")
    }

    /// Create an error when user tries to use Rust's `String` type
    pub fn rust_string_type(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: "Verum uses `Text` instead of `String`".into()
            },
            span,
            ErrorCode::UnexpectedToken,
        ).with_help("replace `String` with `Text`")
    }

    /// Create an error when user tries to use Rust's `Box<T>` type
    pub fn rust_box_type(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: "Verum uses `Heap<T>` instead of `Box<T>`".into()
            },
            span,
            ErrorCode::UnexpectedToken,
        ).with_help("replace `Box<MyType>` with `Heap<MyType>`")
    }

    /// Create an error when user tries to use Rust's `HashMap<K, V>` type
    pub fn rust_hashmap_type(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: "Verum uses `Map<K, V>` instead of `HashMap<K, V>`".into()
            },
            span,
            ErrorCode::UnexpectedToken,
        ).with_help("replace `HashMap<Text, Int>` with `Map<Text, Int>`")
    }

    /// Create an error when user tries to use Rust's `Option<T>` type
    pub fn rust_option_type(span: Span) -> Self {
        Self::with_error_code(
            ParseErrorKind::InvalidSyntax {
                message: "Verum uses `Maybe<T>` instead of `Option<T>`".into()
            },
            span,
            ErrorCode::UnexpectedToken,
        ).with_help("replace `Option<Int>` with `Maybe<Int>`")
    }
}
