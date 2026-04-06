//! Coverage Matrix Representation
//!
//! This module implements the pattern coverage matrix used by the usefulness algorithm.
//! Each row represents a pattern, and each column represents a component of the scrutinee.
//!
//! The matrix enables efficient analysis by allowing:
//! - Specialization (focus on one constructor)
//! - Column extraction (analyze sub-patterns)
//! - Wildcard detection (early termination)

use super::constructors::Constructor;
use crate::context::TypeEnv;
use crate::ty::Type;
use crate::TypeError;
use verum_ast::literal::Literal;
use verum_ast::pattern::{Pattern, PatternKind, VariantPatternData};
use verum_common::{Heap, List, Maybe, Text};

/// A row in the coverage matrix
#[derive(Debug, Clone)]
pub struct PatternRow {
    /// Columns representing pattern components
    pub columns: List<PatternColumn>,

    /// Index in original pattern list
    pub original_index: usize,

    /// Whether this row has a guard (making it conditional)
    pub has_guard: bool,
}

impl PatternRow {
    /// Create a new pattern row
    pub fn new(columns: List<PatternColumn>, original_index: usize, has_guard: bool) -> Self {
        Self {
            columns,
            original_index,
            has_guard,
        }
    }

    /// Check if this row starts with a wildcard
    pub fn is_wildcard(&self) -> bool {
        self.columns
            .first()
            .map(|c| matches!(c, PatternColumn::Wildcard))
            .unwrap_or(true)
    }
}

/// A column in the coverage matrix representing pattern coverage
#[derive(Debug, Clone)]
pub enum PatternColumn {
    /// Covers everything (wildcard, identifier binding)
    Wildcard,

    /// Covers one specific constructor
    Constructor {
        name: Text,
        /// Sub-patterns for constructor arguments
        args: List<PatternColumn>,
    },

    /// Covers a literal value
    Literal(LiteralPattern),

    /// Covers a range of values
    Range {
        start: Option<i128>,
        end: Option<i128>,
        inclusive: bool,
    },

    /// Or-pattern: any of these alternatives
    Or(List<PatternColumn>),

    /// And-pattern: all must match
    And(List<PatternColumn>),

    /// Guarded pattern: inner pattern with guard condition
    /// We track this separately because guards can fail
    Guarded(Box<PatternColumn>),

    /// Tuple pattern with sub-patterns
    Tuple(List<PatternColumn>),

    /// Array pattern with element patterns
    Array(List<PatternColumn>),

    /// Record pattern with field patterns
    Record {
        fields: List<(Text, PatternColumn)>,
        rest: bool,
    },

    /// Reference pattern (dereference)
    Reference {
        mutable: bool,
        inner: Box<PatternColumn>,
    },

    /// Stream pattern: head elements + optional tail binding
    /// Streams are like infinite lists with Nil/Cons structure
    /// Example: `head :: tail` or `a :: b :: rest`
    Stream {
        /// Head element patterns
        head_patterns: List<PatternColumn>,
        /// Optional tail pattern (if bound, e.g., `rest` in `a :: b :: rest`)
        tail: Option<Box<PatternColumn>>,
    },

    /// TypeTest pattern: runtime type check with binding
    /// Example: `x is SomeType` or `x is SomeType as narrowed`
    TypeTest {
        /// The type being tested
        type_name: Text,
        /// Optional binding pattern for the narrowed value
        binding: Option<Box<PatternColumn>>,
    },

    /// Active pattern: user-defined pattern matching
    /// Tracks the function name and extracted bindings
    Active {
        /// Name of the active pattern function
        name: Text,
        /// Extracted bindings from the pattern
        bindings: List<PatternColumn>,
        /// Whether this is a total pattern (returns Bool) or partial (returns Maybe)
        is_total: bool,
    },
}

/// Literal pattern values
#[derive(Debug, Clone)]
pub enum LiteralPattern {
    Int(i64),
    Float(f64),
    Bool(bool),
    Char(char),
    Text(Text),
}

impl PatternColumn {
    /// Convert back to a Pattern (for recursive checking)
    pub fn to_pattern(&self) -> Pattern {
        use verum_ast::span::Span;
        let span = Span::dummy();

        match self {
            PatternColumn::Wildcard => Pattern::wildcard(span),
            PatternColumn::Constructor { name, args } => {
                let path = verum_ast::ty::Path::single(verum_ast::ty::Ident::new(name.clone(), span));
                let data = if args.is_empty() {
                    Maybe::None
                } else {
                    Maybe::Some(VariantPatternData::Tuple(
                        args.iter().map(|c| c.to_pattern()).collect(),
                    ))
                };
                Pattern::new(PatternKind::Variant { path, data }, span)
            }
            PatternColumn::Literal(lit) => {
                let ast_lit = match lit {
                    LiteralPattern::Int(n) => Literal::int(*n as i128, span),
                    LiteralPattern::Float(f) => Literal::float(*f, span),
                    LiteralPattern::Bool(b) => Literal::bool(*b, span),
                    LiteralPattern::Char(c) => Literal::char(*c, span),
                    LiteralPattern::Text(t) => Literal::string(t.clone(), span),
                };
                Pattern::literal(ast_lit)
            }
            PatternColumn::Or(alts) => {
                let patterns = alts.iter().map(|a| a.to_pattern()).collect();
                Pattern::new(PatternKind::Or(patterns), span)
            }
            PatternColumn::And(alts) => {
                let patterns = alts.iter().map(|a| a.to_pattern()).collect();
                Pattern::new(PatternKind::And(patterns), span)
            }
            PatternColumn::Guarded(inner) => inner.to_pattern(),
            PatternColumn::Tuple(elements) => {
                let patterns = elements.iter().map(|e| e.to_pattern()).collect();
                Pattern::new(PatternKind::Tuple(patterns), span)
            }
            PatternColumn::Array(elements) => {
                let patterns = elements.iter().map(|e| e.to_pattern()).collect();
                Pattern::new(PatternKind::Array(patterns), span)
            }
            PatternColumn::Record { fields, rest } => {
                let field_patterns = fields
                    .iter()
                    .map(|(name, col)| {
                        verum_ast::pattern::FieldPattern::new(
                            verum_ast::ty::Ident::new(name.clone(), span),
                            Maybe::Some(col.to_pattern()),
                            span,
                        )
                    })
                    .collect();
                let path = verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    Text::from("_"),
                    span,
                ));
                Pattern::new(
                    PatternKind::Record {
                        path,
                        fields: field_patterns,
                        rest: *rest,
                    },
                    span,
                )
            }
            PatternColumn::Reference { mutable, inner } => Pattern::new(
                PatternKind::Reference {
                    mutable: *mutable,
                    inner: Heap::new(inner.to_pattern()),
                },
                span,
            ),
            PatternColumn::Range { start, end, inclusive } => {
                let start_lit = start.map(|v| Heap::new(Literal::int(v, span)));
                let end_lit = end.map(|v| Heap::new(Literal::int(v, span)));
                Pattern::new(
                    PatternKind::Range {
                        start: start_lit.into(),
                        end: end_lit.into(),
                        inclusive: *inclusive,
                    },
                    span,
                )
            }
            PatternColumn::Stream { head_patterns, tail } => {
                let head = head_patterns.iter().map(|p| p.to_pattern()).collect();
                // rest is optional binding for remaining iterator
                let rest = if tail.is_some() {
                    Maybe::Some(verum_ast::ty::Ident::new(Text::from("_tail"), span))
                } else {
                    Maybe::None
                };
                Pattern::new(
                    PatternKind::Stream {
                        head_patterns: head,
                        rest,
                    },
                    span,
                )
            }
            PatternColumn::TypeTest { type_name, binding: _ } => {
                // TypeTest pattern has binding: Ident and test_type: verum_ast::ty::Type
                let path = verum_ast::ty::Path::single(verum_ast::ty::Ident::new(type_name.clone(), span));
                let test_type = verum_ast::ty::Type::new(
                    verum_ast::ty::TypeKind::Path(path),
                    span,
                );
                Pattern::new(
                    PatternKind::TypeTest {
                        binding: verum_ast::ty::Ident::new(Text::from("_tested"), span),
                        test_type,
                    },
                    span,
                )
            }
            PatternColumn::Active { name, bindings, .. } => {
                Pattern::new(
                    PatternKind::Active {
                        name: verum_ast::ty::Ident::new(name.clone(), span),
                        params: List::new(),
                        bindings: bindings.iter().map(|p| p.to_pattern()).collect(),
                    },
                    span,
                )
            }
        }
    }
}

/// The coverage matrix
#[derive(Debug, Clone)]
pub struct CoverageMatrix {
    /// Rows of the matrix (one per pattern)
    pub rows: Vec<PatternRow>,

    /// Type of the scrutinee
    pub scrutinee_ty: Type,
}

impl CoverageMatrix {
    /// Create a new empty matrix
    pub fn new(scrutinee_ty: Type) -> Self {
        Self {
            rows: Vec::new(),
            scrutinee_ty,
        }
    }

    /// Add a row to the matrix
    pub fn add_row(&mut self, row: PatternRow) {
        self.rows.push(row);
    }

    /// Check if any row is a pure wildcard (covers everything)
    pub fn has_wildcard_row(&self) -> bool {
        self.rows.iter().any(|r| r.is_wildcard() && !r.has_guard)
    }
}

/// Build a coverage matrix from a list of patterns
pub fn build_matrix(
    patterns: &[Pattern],
    scrutinee_ty: &Type,
    _env: &TypeEnv,
) -> Result<CoverageMatrix, TypeError> {
    let mut matrix = CoverageMatrix::new(scrutinee_ty.clone());

    for (idx, pattern) in patterns.iter().enumerate() {
        let (columns, has_guard) = pattern_to_columns(pattern)?;
        let row = PatternRow::new(columns, idx, has_guard);
        matrix.add_row(row);
    }

    Ok(matrix)
}

/// Convert a pattern to matrix columns
fn pattern_to_columns(pattern: &Pattern) -> Result<(List<PatternColumn>, bool), TypeError> {
    let mut has_guard = false;
    let column = pattern_to_column(pattern, &mut has_guard)?;
    Ok((List::from_iter([column]), has_guard))
}

/// Convert a single pattern to a column
fn pattern_to_column(pattern: &Pattern, has_guard: &mut bool) -> Result<PatternColumn, TypeError> {
    match &pattern.kind {
        PatternKind::Wildcard => Ok(PatternColumn::Wildcard),

        PatternKind::Rest => Ok(PatternColumn::Wildcard), // Rest matches anything

        PatternKind::Ident {
            subpattern: Maybe::Some(sub),
            ..
        } => {
            // Identifier with subpattern (@ binding)
            pattern_to_column(sub, has_guard)
        }

        PatternKind::Ident { .. } => Ok(PatternColumn::Wildcard),

        PatternKind::Literal(lit) => {
            let lit_pattern = match &lit.kind {
                verum_ast::literal::LiteralKind::Int(int_lit) => LiteralPattern::Int(int_lit.value as i64),
                verum_ast::literal::LiteralKind::Float(float_lit) => LiteralPattern::Float(float_lit.value),
                verum_ast::literal::LiteralKind::Bool(b) => LiteralPattern::Bool(*b),
                verum_ast::literal::LiteralKind::Char(c) => LiteralPattern::Char(*c),
                verum_ast::literal::LiteralKind::Text(string_lit) => LiteralPattern::Text(Text::from(string_lit.as_str())),
                _ => return Err(TypeError::Other(Text::from("Unsupported literal in pattern"))),
            };
            Ok(PatternColumn::Literal(lit_pattern))
        }

        PatternKind::Tuple(elements) => {
            let columns: Result<List<_>, _> = elements
                .iter()
                .map(|p| pattern_to_column(p, has_guard))
                .collect();
            Ok(PatternColumn::Tuple(columns?))
        }

        PatternKind::Array(elements) => {
            let columns: Result<List<_>, _> = elements
                .iter()
                .map(|p| pattern_to_column(p, has_guard))
                .collect();
            Ok(PatternColumn::Array(columns?))
        }

        PatternKind::Slice { before, rest, after } => {
            // Slice pattern - convert to array-like representation
            let mut columns = List::new();
            for p in before.iter() {
                columns.push(pattern_to_column(p, has_guard)?);
            }
            // Rest pattern represents variable number of elements
            if rest.is_some() {
                columns.push(PatternColumn::Wildcard);
            }
            for p in after.iter() {
                columns.push(pattern_to_column(p, has_guard)?);
            }
            Ok(PatternColumn::Array(columns))
        }

        PatternKind::Record { fields, rest, path } => {
            let field_columns: Result<List<(Text, PatternColumn)>, TypeError> = fields
                .iter()
                .map(|f| {
                    let col = match &f.pattern {
                        Maybe::Some(p) => pattern_to_column(p, has_guard)?,
                        Maybe::None => PatternColumn::Wildcard,
                    };
                    Ok((Text::from(f.name.name.as_str()), col))
                })
                .collect();

            // Get the constructor name from the path
            let ctor_name = path
                .segments
                .last()
                .map(|s| match s {
                    verum_ast::ty::PathSegment::Name(id) => Text::from(id.name.as_str()),
                    _ => Text::from("_"),
                })
                .unwrap_or_else(|| Text::from("_"));

            // For named record patterns, treat as constructor with record fields
            if ctor_name.as_str() != "_" {
                Ok(PatternColumn::Constructor {
                    name: ctor_name,
                    args: field_columns?
                        .iter()
                        .map(|(_, col)| col.clone())
                        .collect(),
                })
            } else {
                Ok(PatternColumn::Record {
                    fields: field_columns?,
                    rest: *rest,
                })
            }
        }

        PatternKind::Variant { path, data } => {
            let name = path
                .segments
                .last()
                .map(|s| match s {
                    verum_ast::ty::PathSegment::Name(id) => Text::from(id.name.as_str()),
                    _ => Text::from("_"),
                })
                .unwrap_or_else(|| Text::from("_"));

            let args = match data {
                Maybe::None => List::new(),
                Maybe::Some(VariantPatternData::Tuple(patterns)) => {
                    let cols: Result<List<_>, _> = patterns
                        .iter()
                        .map(|p| pattern_to_column(p, has_guard))
                        .collect();
                    cols?
                }
                Maybe::Some(VariantPatternData::Record { fields, .. }) => {
                    let cols: Result<List<_>, _> = fields
                        .iter()
                        .map(|f| {
                            match &f.pattern {
                                Maybe::Some(p) => pattern_to_column(p, has_guard),
                                Maybe::None => Ok(PatternColumn::Wildcard),
                            }
                        })
                        .collect();
                    cols?
                }
            };

            Ok(PatternColumn::Constructor { name, args })
        }

        PatternKind::Or(alternatives) => {
            let cols: Result<List<_>, _> = alternatives
                .iter()
                .map(|p| pattern_to_column(p, has_guard))
                .collect();
            Ok(PatternColumn::Or(cols?))
        }

        PatternKind::And(conjuncts) => {
            let cols: Result<List<_>, _> = conjuncts
                .iter()
                .map(|p| pattern_to_column(p, has_guard))
                .collect();
            Ok(PatternColumn::And(cols?))
        }

        PatternKind::Guard { pattern, .. } => {
            *has_guard = true;
            let inner = pattern_to_column(pattern, has_guard)?;
            Ok(PatternColumn::Guarded(Box::new(inner)))
        }

        PatternKind::Reference { mutable, inner } => {
            let inner_col = pattern_to_column(inner, has_guard)?;
            Ok(PatternColumn::Reference {
                mutable: *mutable,
                inner: Box::new(inner_col),
            })
        }

        PatternKind::Range {
            start,
            end,
            inclusive,
        } => {
            let start_val = start.as_ref().and_then(|l| match &l.kind {
                verum_ast::literal::LiteralKind::Int(int_lit) => Some(int_lit.value),
                _ => None,
            });
            let end_val = end.as_ref().and_then(|l| match &l.kind {
                verum_ast::literal::LiteralKind::Int(int_lit) => Some(int_lit.value),
                _ => None,
            });
            Ok(PatternColumn::Range {
                start: start_val,
                end: end_val,
                inclusive: *inclusive,
            })
        }

        PatternKind::Paren(inner) => pattern_to_column(inner, has_guard),        PatternKind::View { pattern, .. } => {
            // Deprecated view patterns - just check inner pattern
            pattern_to_column(pattern, has_guard)
        }

        PatternKind::Active { name, bindings, .. } => {
            // Active patterns are user-defined pattern functions
            // They can be total (returns Bool) or partial (returns Maybe<T>)
            let pattern_name = Text::from(name.name.as_str());

            let is_total = bindings.is_empty();
            *has_guard = true; // Active patterns may not match

            let binding_cols: Result<List<_>, _> = bindings
                .iter()
                .map(|p| pattern_to_column(p, has_guard))
                .collect();

            Ok(PatternColumn::Active {
                name: pattern_name,
                bindings: binding_cols?,
                is_total,
            })
        }

        PatternKind::TypeTest { test_type, binding } => {
            // TypeTest patterns perform runtime type checks.
            // Mark as guarded here since we don't know the scrutinee type at this level.
            // The specialize_matrix function will override has_guard to false when the
            // TypeTest matches a specific variant constructor (e.g., `x is Dog` for
            // type `Animal = Dog | Cat | Bird`), enabling proper exhaustiveness detection.
            *has_guard = true;

            // test_type is verum_ast::ty::Type which has kind: TypeKind
            let type_name = match &test_type.kind {
                verum_ast::ty::TypeKind::Path(path) => path
                    .segments
                    .last()
                    .map(|s| match s {
                        verum_ast::ty::PathSegment::Name(id) => Text::from(id.name.as_str()),
                        _ => Text::from("_"),
                    })
                    .unwrap_or_else(|| Text::from("_")),
                verum_ast::ty::TypeKind::Generic { base, .. } => {
                    // Extract name from base type
                    match &base.kind {
                        verum_ast::ty::TypeKind::Path(path) => path
                            .segments
                            .last()
                            .map(|s| match s {
                                verum_ast::ty::PathSegment::Name(id) => Text::from(id.name.as_str()),
                                _ => Text::from("_"),
                            })
                            .unwrap_or_else(|| Text::from("_")),
                        _ => Text::from("_"),
                    }
                }
                _ => Text::from("_"),
            };

            // Binding is the identifier to which the narrowed value is bound
            let binding_col = Some(Box::new(PatternColumn::Wildcard));
            let _ = binding; // Binding is just an identifier name, not a pattern

            Ok(PatternColumn::TypeTest {
                type_name,
                binding: binding_col,
            })
        }

        PatternKind::Stream { head_patterns, rest } => {
            // Stream patterns decompose iterators/streams
            // Structure: stream[a, b, ...rest]
            let head_cols: Result<List<_>, _> = head_patterns
                .iter()
                .map(|p| pattern_to_column(p, has_guard))
                .collect();

            // rest is Maybe<Ident> - if present, the remaining iterator is bound
            let tail_col = match rest.as_ref() {
                Maybe::Some(_) => Some(Box::new(PatternColumn::Wildcard)),
                Maybe::None => None,
            };

            Ok(PatternColumn::Stream {
                head_patterns: head_cols?,
                tail: tail_col,
            })
        }

        PatternKind::Cons { head, tail } => {
            // Cons pattern: head :: tail - list decomposition
            let head_col = pattern_to_column(head, has_guard)?;
            let tail_col = pattern_to_column(tail, has_guard)?;
            Ok(PatternColumn::Constructor {
                name: Text::from("::"),
                args: List::from(vec![head_col, tail_col]),
            })
        }
    }
}

/// Specialize the matrix for a specific constructor
///
/// This filters rows that match the constructor and expands their sub-patterns.
pub fn specialize_matrix(matrix: &CoverageMatrix, ctor: &Constructor) -> CoverageMatrix {
    let mut specialized = CoverageMatrix::new(matrix.scrutinee_ty.clone());

    for row in &matrix.rows {
        if let Some(first) = row.columns.first() {
            match first {
                PatternColumn::Wildcard => {
                    // Wildcard matches all constructors
                    let mut new_columns: List<PatternColumn> =
                        (0..ctor.arg_types.len()).map(|_| PatternColumn::Wildcard).collect();
                    for col in row.columns.iter().skip(1) {
                        new_columns.push(col.clone());
                    }
                    specialized.add_row(PatternRow::new(
                        new_columns,
                        row.original_index,
                        row.has_guard,
                    ));
                }
                PatternColumn::Constructor { name, args } if name == &ctor.name => {
                    // Matching constructor - expand arguments
                    let mut new_columns = args.clone();
                    for col in row.columns.iter().skip(1) {
                        new_columns.push(col.clone());
                    }
                    specialized.add_row(PatternRow::new(
                        new_columns,
                        row.original_index,
                        row.has_guard,
                    ));
                }
                PatternColumn::Or(alts) => {
                    // Check if any alternative matches
                    for alt in alts.iter() {
                        if matches_constructor(alt, ctor) {
                            let expanded = expand_alternative(alt, ctor);
                            let mut new_columns = expanded;
                            for col in row.columns.iter().skip(1) {
                                new_columns.push(col.clone());
                            }
                            specialized.add_row(PatternRow::new(
                                new_columns,
                                row.original_index,
                                row.has_guard,
                            ));
                        }
                    }
                }
                PatternColumn::Guarded(inner) => {
                    // Process inner pattern but mark as guarded
                    if matches_constructor(inner, ctor) {
                        let expanded = expand_alternative(inner, ctor);
                        let mut new_columns = expanded;
                        for col in row.columns.iter().skip(1) {
                            new_columns.push(col.clone());
                        }
                        specialized.add_row(PatternRow::new(
                            new_columns,
                            row.original_index,
                            true, // Keep guard flag
                        ));
                    }
                }
                PatternColumn::And(conjuncts) => {
                    // And pattern: all conjuncts must match. For exhaustiveness,
                    // find the most specific conjunct that matches this constructor
                    // and use it for specialization. Wildcards/idents are least specific.
                    if let Some(specific) = find_most_specific_conjunct(conjuncts, ctor) {
                        let expanded = expand_alternative(specific, ctor);
                        let mut new_columns = expanded;
                        for col in row.columns.iter().skip(1) {
                            new_columns.push(col.clone());
                        }
                        specialized.add_row(PatternRow::new(
                            new_columns,
                            row.original_index,
                            row.has_guard,
                        ));
                    }
                }
                // Bool literal patterns match bool constructors "true"/"false"
                PatternColumn::Literal(LiteralPattern::Bool(b)) => {
                    let lit_name = if *b { "true" } else { "false" };
                    if ctor.name.as_str() == lit_name || ctor.is_default {
                        // Bool literals are nullary - no sub-patterns
                        let mut new_columns = List::new();
                        for col in row.columns.iter().skip(1) {
                            new_columns.push(col.clone());
                        }
                        specialized.add_row(PatternRow::new(
                            new_columns,
                            row.original_index,
                            row.has_guard,
                        ));
                    }
                }
                // Tuple patterns match the tuple constructor "()"
                PatternColumn::Tuple(elements)
                    if ctor.name.as_str() == "()" || ctor.is_default =>
                {
                    let mut new_columns = elements.clone();
                    for col in row.columns.iter().skip(1) {
                        new_columns.push(col.clone());
                    }
                    specialized.add_row(PatternRow::new(
                        new_columns,
                        row.original_index,
                        row.has_guard,
                    ));
                }
                // TypeTest patterns: `x is Dog` covers the `Dog` constructor
                PatternColumn::TypeTest { type_name, .. }
                    if type_name.as_str() == ctor.name.as_str() || ctor.is_default =>
                {
                    let mut new_columns: List<PatternColumn> =
                        (0..ctor.arg_types.len()).map(|_| PatternColumn::Wildcard).collect();
                    for col in row.columns.iter().skip(1) {
                        new_columns.push(col.clone());
                    }
                    // When the TypeTest exactly matches a named constructor (not the
                    // default wildcard), it provides definitive coverage (e.g.,
                    // `x is Dog` covers the `Dog` variant). For the default constructor,
                    // preserve the guarded status since the runtime check may fail.
                    let is_exact_match = type_name.as_str() == ctor.name.as_str();
                    specialized.add_row(PatternRow::new(
                        new_columns,
                        row.original_index,
                        if is_exact_match { false } else { row.has_guard },
                    ));
                }
                _ => {
                    // Other patterns don't match this constructor
                }
            }
        }
    }

    specialized
}

/// Find the most specific conjunct in an And pattern that matches a constructor.
/// Prefers Constructor/Literal/Tuple patterns over Wildcard/Ident.
fn find_most_specific_conjunct<'a>(
    conjuncts: &'a List<PatternColumn>,
    ctor: &Constructor,
) -> Option<&'a PatternColumn> {
    let mut best: Option<&'a PatternColumn> = None;
    for conj in conjuncts.iter() {
        if matches_constructor(conj, ctor) {
            match conj {
                PatternColumn::Wildcard => {
                    if best.is_none() {
                        best = Some(conj);
                    }
                }
                _ => {
                    // Non-wildcard is more specific
                    best = Some(conj);
                }
            }
        }
    }
    best
}

/// Check if a pattern column matches a constructor
fn matches_constructor(col: &PatternColumn, ctor: &Constructor) -> bool {
    match col {
        PatternColumn::Wildcard => true,
        PatternColumn::Constructor { name, .. } => name == &ctor.name || ctor.is_default,
        PatternColumn::Or(alts) => alts.iter().any(|a| matches_constructor(a, ctor)),
        PatternColumn::Guarded(inner) => matches_constructor(inner, ctor),
        PatternColumn::And(conjuncts) => conjuncts.iter().any(|c| matches_constructor(c, ctor)),
        // Bool literals match bool constructors "true"/"false"
        PatternColumn::Literal(LiteralPattern::Bool(b)) => {
            let lit_name = if *b { "true" } else { "false" };
            ctor.name.as_str() == lit_name || ctor.is_default
        }
        PatternColumn::Literal(_) if ctor.is_default => true,
        PatternColumn::Range { .. } if ctor.is_default => true,
        // Tuple patterns match the tuple constructor "()"
        PatternColumn::Tuple(_) => ctor.name.as_str() == "()" || ctor.is_default,
        // Stream patterns match stream constructors structurally
        PatternColumn::Stream { head_patterns, .. } => {
            // Empty head matches the nullary constructor, non-empty matches the one with args
            if head_patterns.is_empty() {
                ctor.arg_types.is_empty() || ctor.is_default
            } else {
                !ctor.arg_types.is_empty() || ctor.is_default
            }
        }
        // TypeTest: when the type_name matches a constructor name, treat it
        // as covering that constructor (e.g., `x is Dog` covers the `Dog` variant).
        // This enables exhaustiveness checking for variant type tests.
        PatternColumn::TypeTest { type_name, .. } => {
            type_name.as_str() == ctor.name.as_str() || ctor.is_default
        }
        // Active patterns are user-defined - conservative match
        PatternColumn::Active { is_total, .. } => {
            // Total active patterns (Bool) can match anything
            // Partial patterns (Maybe) match conservatively
            *is_total || ctor.is_default
        }
        _ => false,
    }
}

/// Expand a pattern column for a constructor
fn expand_alternative(col: &PatternColumn, ctor: &Constructor) -> List<PatternColumn> {
    match col {
        PatternColumn::Wildcard => {
            (0..ctor.arg_types.len())
                .map(|_| PatternColumn::Wildcard)
                .collect()
        }
        PatternColumn::Constructor { args, .. } => args.clone(),
        PatternColumn::Guarded(inner) => expand_alternative(inner, ctor),
        PatternColumn::And(conjuncts) => {
            // Use the most specific conjunct for expansion
            if let Some(specific) = find_most_specific_conjunct(conjuncts, ctor) {
                expand_alternative(specific, ctor)
            } else {
                (0..ctor.arg_types.len())
                    .map(|_| PatternColumn::Wildcard)
                    .collect()
            }
        }
        // Bool literals: nullary, no sub-patterns to expand
        PatternColumn::Literal(LiteralPattern::Bool(_)) => List::new(),
        // Tuple patterns: expand to element patterns (like constructor args)
        PatternColumn::Tuple(elements) => elements.clone(),
        // Stream patterns: expand head and tail for Cons constructor
        PatternColumn::Stream { head_patterns, tail } => {
            if !ctor.arg_types.is_empty() && !head_patterns.is_empty() {
                // Cons(head, tail) - first element + rest
                let mut result = List::new();
                if let Some(first) = head_patterns.first() {
                    result.push(first.clone());
                }
                // Remaining head elements become a nested stream
                if head_patterns.len() > 1 {
                    result.push(PatternColumn::Stream {
                        head_patterns: head_patterns.iter().skip(1).cloned().collect(),
                        tail: tail.clone(),
                    });
                } else if let Some(t) = tail {
                    result.push(t.as_ref().clone());
                } else {
                    result.push(PatternColumn::Wildcard);
                }
                result
            } else {
                // Nil or unknown constructor
                List::new()
            }
        }
        // TypeTest and Active patterns don't expand to constructor args
        PatternColumn::TypeTest { .. } | PatternColumn::Active { .. } => {
            (0..ctor.arg_types.len())
                .map(|_| PatternColumn::Wildcard)
                .collect()
        }
        _ => List::new(),
    }
}

/// Extract a specific column from the matrix
pub fn extract_column(matrix: &CoverageMatrix, col_idx: usize) -> Vec<PatternColumn> {
    matrix
        .rows
        .iter()
        .filter_map(|row| row.columns.get(col_idx).cloned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wildcard_pattern() {
        let span = verum_ast::span::Span::dummy();
        let pattern = Pattern::wildcard(span);
        let (columns, has_guard) = pattern_to_columns(&pattern).unwrap();
        assert_eq!(columns.len(), 1);
        assert!(!has_guard);
        assert!(matches!(columns[0], PatternColumn::Wildcard));
    }

    #[test]
    fn test_variant_pattern() {
        let span = verum_ast::span::Span::dummy();
        let path = verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
            Text::from("Some"),
            span,
        ));
        let pattern = Pattern::new(
            PatternKind::Variant {
                path,
                data: Maybe::Some(VariantPatternData::Tuple(List::from_iter([
                    Pattern::wildcard(span),
                ]))),
            },
            span,
        );
        let (columns, _) = pattern_to_columns(&pattern).unwrap();
        assert!(matches!(columns[0], PatternColumn::Constructor { .. }));
    }
}
