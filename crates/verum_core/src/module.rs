//! Module-level IR.
//!
//! [`IrModule`] is the top-level read-only view a verification pass
//! consumes. It holds the lowered view of every function, theorem,
//! lemma, corollary, axiom, and type declaration in one compilation
//! unit, with stable accessor APIs so passes don't re-walk the raw
//! AST for pipeline-shared facts.

use serde::{Deserialize, Serialize};
use verum_common::{List, Maybe, Text};
use verum_common::span::Span;

use crate::expr::IrExpr;
use crate::ty::IrType;

/// An IR module — a single compilation unit's lowered contents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrModule {
    /// Fully-qualified module path (`core.math.bits`).
    pub path: Text,
    /// Type declarations (variants, records, aliases, newtypes).
    pub types: List<IrTypeDecl>,
    /// Function declarations.
    pub functions: List<IrFunction>,
    /// Theorem / lemma / corollary / axiom declarations.
    pub theorems: List<IrTheorem>,
    /// Source span of the module declaration.
    pub span: Span,
}

/// A type declaration in the IR.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrTypeDecl {
    /// Unqualified type name.
    pub name: Text,
    /// Generic parameter names.
    pub generics: List<Text>,
    /// Kind-specific data.
    pub variant_kind: IrVariantKind,
    /// Source span.
    pub span: Span,
}

/// Shape of a type declaration's body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IrVariantKind {
    /// Record with named fields.
    Record {
        /// Field list as `(name, type)` pairs.
        fields: List<(Text, IrType)>,
    },
    /// Sum type with constructor list.
    Variant {
        /// Constructors — `(name, payload-types)`.
        constructors: List<(Text, List<IrType>)>,
    },
    /// Newtype — a single-constructor wrapper.
    Newtype {
        /// The wrapped type.
        inner: IrType,
    },
    /// Alias (`type X is Y;`).
    Alias {
        /// The aliased target.
        target: IrType,
    },
    /// Refinement alias (`type X is Y { self op K };`).
    Refined {
        /// The base type.
        base: IrType,
        /// The refinement predicate (binder convention: `self`).
        predicate: IrExpr,
    },
    /// Protocol / trait.
    Protocol {
        /// Protocol method signatures (`(name, fn-type)`).
        methods: List<(Text, IrType)>,
    },
}

/// A function declaration in the IR.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrFunction {
    /// Unqualified function name.
    pub name: Text,
    /// Parameters as `(name, type)` pairs.
    pub params: List<(Text, IrType)>,
    /// Return type.
    pub return_type: IrType,
    /// Preconditions.
    pub requires: List<IrExpr>,
    /// Postconditions.
    pub ensures: List<IrExpr>,
    /// Optional body as an IR expression (`None` for declared-only
    /// functions like `fn is_prime(n: Int) -> Bool;`).
    pub body: Maybe<IrExpr>,
    /// Source span.
    pub span: Span,
}

/// A theorem / lemma / corollary / axiom in the IR.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrTheorem {
    /// Unqualified name.
    pub name: Text,
    /// Kind of declaration.
    pub kind: IrTheoremKind,
    /// Parameters.
    pub params: List<(Text, IrType)>,
    /// Preconditions.
    pub requires: List<IrExpr>,
    /// Postconditions.
    pub ensures: List<IrExpr>,
    /// The stated proposition.
    pub proposition: IrExpr,
    /// Whether the theorem has a proof body (false for axioms).
    pub has_proof: bool,
    /// Framework attributions for axioms / framework-conditional
    /// theorems, e.g. `("baez_dolan", "Aut(𝕆) = G₂")`.
    pub framework_tags: List<(Text, Text)>,
    /// Source span.
    pub span: Span,
}

/// Kind of theorem-family declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IrTheoremKind {
    /// A regular theorem.
    Theorem,
    /// A lemma (lowered the same as theorem; tag kept for UX).
    Lemma,
    /// A corollary.
    Corollary,
    /// An axiom (no proof; registered as a trusted fact).
    Axiom,
}

impl IrModule {
    /// Build an empty module shell at the given path.
    #[must_use]
    pub fn empty(path: impl Into<Text>, span: Span) -> Self {
        Self {
            path: path.into(),
            types: List::new(),
            functions: List::new(),
            theorems: List::new(),
            span,
        }
    }

    /// Look up a variant type's constructor list by name. Returns
    /// `None` when the name isn't declared in this module, or the
    /// declaration isn't a variant.
    #[must_use]
    pub fn variant_constructors(&self, name: &Text) -> Option<Vec<Text>> {
        for td in &self.types {
            if td.name == *name {
                if let IrVariantKind::Variant { constructors } = &td.variant_kind {
                    return Some(
                        constructors.iter().map(|(n, _)| n.clone()).collect(),
                    );
                }
                return None;
            }
        }
        None
    }

    /// Count theorems by kind. Useful for auditing module coverage.
    #[must_use]
    pub fn theorem_count_by_kind(&self, kind: IrTheoremKind) -> usize {
        self.theorems.iter().filter(|t| t.kind == kind).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_common::span::Span;

    #[test]
    fn empty_module_has_no_entries() {
        let m = IrModule::empty("core.test", Span::dummy());
        assert_eq!(m.types.len(), 0);
        assert_eq!(m.functions.len(), 0);
        assert_eq!(m.theorems.len(), 0);
    }

    #[test]
    fn variant_lookup_returns_ctor_names() {
        let mut m = IrModule::empty("core.test", Span::dummy());
        m.types.push(IrTypeDecl {
            name: Text::from("Color"),
            generics: List::new(),
            variant_kind: IrVariantKind::Variant {
                constructors: List::from_iter([
                    (Text::from("Red"), List::new()),
                    (Text::from("Green"), List::new()),
                    (Text::from("Blue"), List::new()),
                ]),
            },
            span: Span::dummy(),
        });
        let ctors = m
            .variant_constructors(&Text::from("Color"))
            .expect("Color registered");
        assert_eq!(ctors.len(), 3);
        assert_eq!(ctors[0].as_str(), "Red");
        assert_eq!(ctors[2].as_str(), "Blue");
    }

    #[test]
    fn variant_lookup_misses_return_none() {
        let m = IrModule::empty("core.test", Span::dummy());
        assert!(m
            .variant_constructors(&Text::from("Absent"))
            .is_none());
    }

    #[test]
    fn theorem_count_filter_works() {
        let mut m = IrModule::empty("core.test", Span::dummy());
        m.theorems.push(IrTheorem {
            name: Text::from("t1"),
            kind: IrTheoremKind::Theorem,
            params: List::new(),
            requires: List::new(),
            ensures: List::new(),
            proposition: crate::expr::IrExpr::new(
                crate::expr::IrExprKind::BoolLit(true),
                None,
                Span::dummy(),
            ),
            has_proof: true,
            framework_tags: List::new(),
            span: Span::dummy(),
        });
        m.theorems.push(IrTheorem {
            name: Text::from("a1"),
            kind: IrTheoremKind::Axiom,
            params: List::new(),
            requires: List::new(),
            ensures: List::new(),
            proposition: crate::expr::IrExpr::new(
                crate::expr::IrExprKind::BoolLit(true),
                None,
                Span::dummy(),
            ),
            has_proof: false,
            framework_tags: List::new(),
            span: Span::dummy(),
        });
        assert_eq!(m.theorem_count_by_kind(IrTheoremKind::Theorem), 1);
        assert_eq!(m.theorem_count_by_kind(IrTheoremKind::Axiom), 1);
        assert_eq!(m.theorem_count_by_kind(IrTheoremKind::Lemma), 0);
    }
}
