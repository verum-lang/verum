//! Meta pattern IR
//!
//! This module defines the intermediate representation for meta patterns
//! used in match expressions during compile-time execution.
//!
//! ## Industrial-Grade Pattern Support
//!
//! This module provides comprehensive pattern matching for meta evaluation:
//! - Wildcard, literal, and identifier patterns
//! - Tuple, array, and record patterns
//! - Variant (enum) patterns
//! - Range patterns (inclusive and exclusive)
//! - Rest patterns for slice matching
//! - And/Or patterns for complex matching
//! - Reference patterns
//! - Subpattern binding (@ syntax)
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).

use verum_ast::MetaValue;
use verum_common::well_known_types::type_names;
use verum_common::{Heap, List, Maybe, Text};

/// Patterns for meta matching
///
/// Comprehensive pattern support for compile-time evaluation.
#[derive(Debug, Clone, PartialEq)]
pub enum MetaPattern {
    /// Wildcard pattern: `_`
    Wildcard,

    /// Literal pattern: `42`, `"hello"`, `true`
    Literal(MetaValue),

    /// Variable binding: `x`, `name`
    Ident(Text),

    /// Identifier with subpattern: `x @ Some(inner)`
    IdentAt {
        name: Text,
        subpattern: Heap<MetaPattern>,
    },

    /// Tuple pattern: `(a, b, c)`
    Tuple(List<MetaPattern>),

    /// Array pattern: `[a, b, c]`
    Array(List<MetaPattern>),

    /// Slice pattern with rest: `[head, .., tail]` or `[first, ..rest]`
    Slice {
        /// Patterns before the rest
        before: List<MetaPattern>,
        /// Optional binding for rest elements
        rest: Maybe<Text>,
        /// Patterns after the rest
        after: List<MetaPattern>,
    },

    /// Record pattern: `Point { x, y }` or `Point { x: px, y: py }`
    Record {
        /// Type/constructor name
        name: Text,
        /// Field patterns (field_name, pattern)
        fields: List<(Text, MetaPattern)>,
        /// Whether to ignore extra fields with `..`
        rest: bool,
    },

    /// Variant pattern: `Some(x)` or `None`
    Variant {
        /// Variant name (e.g., "Some", "None", "Ok", "Err")
        name: Text,
        /// Variant data pattern (None for unit variants)
        data: Maybe<Heap<MetaPattern>>,
    },

    /// Range pattern: `1..10` or `1..=10`
    Range {
        /// Start of range (None for unbounded start)
        start: Maybe<MetaValue>,
        /// End of range (None for unbounded end)
        end: Maybe<MetaValue>,
        /// Whether the end is inclusive
        inclusive: bool,
    },

    /// Rest pattern: `..` or `..rest`
    Rest(Maybe<Text>),

    /// Or pattern: `a | b | c`
    Or(List<MetaPattern>),

    /// And pattern: `p1 and p2` (both must match)
    And(List<MetaPattern>),

    /// Reference pattern: `&x` or `&mut x`
    Reference {
        mutable: bool,
        inner: Heap<MetaPattern>,
    },

    /// Type test pattern: `x: Type`
    TypeTest {
        /// Binding name
        name: Text,
        /// Expected type name
        type_name: Text,
    },
}

impl MetaPattern {
    /// Create a wildcard pattern
    #[inline]
    pub fn wildcard() -> Self {
        MetaPattern::Wildcard
    }

    /// Create a literal pattern
    #[inline]
    pub fn literal(value: MetaValue) -> Self {
        MetaPattern::Literal(value)
    }

    /// Create an identifier binding pattern
    #[inline]
    pub fn ident(name: Text) -> Self {
        MetaPattern::Ident(name)
    }

    /// Create an identifier with subpattern (@ binding)
    #[inline]
    pub fn ident_at(name: Text, subpattern: MetaPattern) -> Self {
        MetaPattern::IdentAt {
            name,
            subpattern: Heap::new(subpattern),
        }
    }

    /// Create a tuple pattern
    #[inline]
    pub fn tuple(patterns: List<MetaPattern>) -> Self {
        MetaPattern::Tuple(patterns)
    }

    /// Create an array pattern
    #[inline]
    pub fn array(patterns: List<MetaPattern>) -> Self {
        MetaPattern::Array(patterns)
    }

    /// Create a slice pattern
    #[inline]
    pub fn slice(before: List<MetaPattern>, rest: Maybe<Text>, after: List<MetaPattern>) -> Self {
        MetaPattern::Slice { before, rest, after }
    }

    /// Create a record pattern
    #[inline]
    pub fn record(name: Text, fields: List<(Text, MetaPattern)>, rest: bool) -> Self {
        MetaPattern::Record { name, fields, rest }
    }

    /// Create a variant pattern
    #[inline]
    pub fn variant(name: Text, data: Maybe<MetaPattern>) -> Self {
        MetaPattern::Variant {
            name,
            data: data.map(|p| Heap::new(p)),
        }
    }

    /// Create a range pattern
    #[inline]
    pub fn range(start: Maybe<MetaValue>, end: Maybe<MetaValue>, inclusive: bool) -> Self {
        MetaPattern::Range { start, end, inclusive }
    }

    /// Create a rest pattern
    #[inline]
    pub fn rest(binding: Maybe<Text>) -> Self {
        MetaPattern::Rest(binding)
    }

    /// Create an or pattern
    #[inline]
    pub fn or(patterns: List<MetaPattern>) -> Self {
        MetaPattern::Or(patterns)
    }

    /// Create an and pattern
    #[inline]
    pub fn and(patterns: List<MetaPattern>) -> Self {
        MetaPattern::And(patterns)
    }

    /// Create a reference pattern
    #[inline]
    pub fn reference(mutable: bool, inner: MetaPattern) -> Self {
        MetaPattern::Reference {
            mutable,
            inner: Heap::new(inner),
        }
    }

    /// Create a type test pattern
    #[inline]
    pub fn type_test(name: Text, type_name: Text) -> Self {
        MetaPattern::TypeTest { name, type_name }
    }

    /// Check if pattern is irrefutable (always matches)
    pub fn is_irrefutable(&self) -> bool {
        match self {
            MetaPattern::Wildcard | MetaPattern::Ident(_) | MetaPattern::Rest(_) => true,
            MetaPattern::IdentAt { subpattern, .. } => subpattern.is_irrefutable(),
            MetaPattern::Literal(_) | MetaPattern::Range { .. } | MetaPattern::TypeTest { .. } => false,
            MetaPattern::Tuple(pats) | MetaPattern::Array(pats) => {
                pats.iter().all(|p| p.is_irrefutable())
            }
            MetaPattern::Slice { before, after, .. } => {
                before.iter().all(|p| p.is_irrefutable())
                    && after.iter().all(|p| p.is_irrefutable())
            }
            MetaPattern::Record { fields, rest, .. } => {
                *rest || fields.iter().all(|(_, p)| p.is_irrefutable())
            }
            MetaPattern::Variant { .. } => false, // Variants are generally refutable
            MetaPattern::Or(pats) => pats.iter().any(|p| p.is_irrefutable()),
            MetaPattern::And(pats) => pats.iter().all(|p| p.is_irrefutable()),
            MetaPattern::Reference { inner, .. } => inner.is_irrefutable(),
        }
    }

    /// Get all bound identifiers in this pattern
    pub fn bound_identifiers(&self) -> List<Text> {
        let mut names = List::new();
        self.collect_identifiers(&mut names);
        names
    }

    fn collect_identifiers(&self, names: &mut List<Text>) {
        match self {
            MetaPattern::Ident(name) => names.push(name.clone()),
            MetaPattern::IdentAt { name, subpattern } => {
                names.push(name.clone());
                subpattern.collect_identifiers(names);
            }
            MetaPattern::Tuple(pats) | MetaPattern::Array(pats) => {
                for pat in pats {
                    pat.collect_identifiers(names);
                }
            }
            MetaPattern::Slice { before, rest, after } => {
                for pat in before {
                    pat.collect_identifiers(names);
                }
                if let Maybe::Some(name) = rest {
                    names.push(name.clone());
                }
                for pat in after {
                    pat.collect_identifiers(names);
                }
            }
            MetaPattern::Record { fields, .. } => {
                for (_, pat) in fields {
                    pat.collect_identifiers(names);
                }
            }
            MetaPattern::Variant { data, .. } => {
                if let Maybe::Some(pat) = data {
                    pat.collect_identifiers(names);
                }
            }
            MetaPattern::Rest(binding) => {
                if let Maybe::Some(name) = binding {
                    names.push(name.clone());
                }
            }
            MetaPattern::Or(pats) => {
                // For or patterns, only collect from first branch
                // (all branches should bind same names)
                if let Some(first) = pats.first() {
                    first.collect_identifiers(names);
                }
            }
            MetaPattern::And(pats) => {
                for pat in pats {
                    pat.collect_identifiers(names);
                }
            }
            MetaPattern::Reference { inner, .. } => {
                inner.collect_identifiers(names);
            }
            MetaPattern::TypeTest { name, .. } => {
                names.push(name.clone());
            }
            MetaPattern::Wildcard | MetaPattern::Literal(_) | MetaPattern::Range { .. } => {}
        }
    }

    /// Check if this pattern could match a given value type
    pub fn could_match_type(&self, type_name: &str) -> bool {
        match self {
            MetaPattern::Wildcard | MetaPattern::Ident(_) | MetaPattern::IdentAt { .. } => true,
            MetaPattern::Literal(v) => v.type_name().as_str() == type_name,
            MetaPattern::Tuple(_) => type_name == "Tuple" || type_name.starts_with('('),
            MetaPattern::Array(_) | MetaPattern::Slice { .. } => {
                type_name == type_names::ARRAY || type_name == type_names::LIST || type_name.starts_with('[')
            }
            MetaPattern::Record { name, .. } => name.as_str() == type_name,
            MetaPattern::Variant { .. } => true, // Depends on enum definition
            MetaPattern::Range { .. } => matches!(type_name, type_names::INT | "UInt" | type_names::CHAR),
            MetaPattern::Rest(_) => true,
            MetaPattern::Or(pats) => pats.iter().any(|p| p.could_match_type(type_name)),
            MetaPattern::And(pats) => pats.iter().all(|p| p.could_match_type(type_name)),
            MetaPattern::Reference { .. } => type_name.starts_with('&'),
            MetaPattern::TypeTest { type_name: expected, .. } => expected.as_str() == type_name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wildcard_is_irrefutable() {
        assert!(MetaPattern::wildcard().is_irrefutable());
    }

    #[test]
    fn test_ident_is_irrefutable() {
        assert!(MetaPattern::ident(Text::from("x")).is_irrefutable());
    }

    #[test]
    fn test_literal_is_refutable() {
        assert!(!MetaPattern::literal(MetaValue::Int(42)).is_irrefutable());
    }

    #[test]
    fn test_tuple_irrefutability() {
        let irrefutable = MetaPattern::tuple(vec![
            MetaPattern::wildcard(),
            MetaPattern::ident(Text::from("x")),
        ].into());
        assert!(irrefutable.is_irrefutable());

        let refutable = MetaPattern::tuple(vec![
            MetaPattern::wildcard(),
            MetaPattern::literal(MetaValue::Int(1)),
        ].into());
        assert!(!refutable.is_irrefutable());
    }

    #[test]
    fn test_or_irrefutability() {
        // Or is irrefutable if any branch is irrefutable
        let irrefutable = MetaPattern::or(vec![
            MetaPattern::literal(MetaValue::Int(1)),
            MetaPattern::wildcard(),
        ].into());
        assert!(irrefutable.is_irrefutable());
    }

    #[test]
    fn test_bound_identifiers() {
        let pat = MetaPattern::tuple(vec![
            MetaPattern::ident(Text::from("a")),
            MetaPattern::ident(Text::from("b")),
            MetaPattern::wildcard(),
        ].into());
        let names = pat.bound_identifiers();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&Text::from("a")));
        assert!(names.contains(&Text::from("b")));
    }

    #[test]
    fn test_variant_pattern() {
        let some_pat = MetaPattern::variant(
            Text::from("Some"),
            Maybe::Some(MetaPattern::ident(Text::from("x"))),
        );
        let names = some_pat.bound_identifiers();
        assert_eq!(names.len(), 1);
        assert!(names.contains(&Text::from("x")));
    }

    #[test]
    fn test_slice_pattern() {
        let slice_pat = MetaPattern::slice(
            vec![MetaPattern::ident(Text::from("head"))].into(),
            Maybe::Some(Text::from("rest")),
            vec![MetaPattern::ident(Text::from("tail"))].into(),
        );
        let names = slice_pat.bound_identifiers();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&Text::from("head")));
        assert!(names.contains(&Text::from("rest")));
        assert!(names.contains(&Text::from("tail")));
    }

    #[test]
    fn test_record_pattern() {
        let rec_pat = MetaPattern::record(
            Text::from("Point"),
            vec![
                (Text::from("x"), MetaPattern::ident(Text::from("px"))),
                (Text::from("y"), MetaPattern::ident(Text::from("py"))),
            ].into(),
            false,
        );
        let names = rec_pat.bound_identifiers();
        assert_eq!(names.len(), 2);
    }
}
