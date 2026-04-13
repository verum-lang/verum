//! Polymorphic Kinds — kind variables, kind unification.
//!
//! In Hindley-Milner-style type systems, the *kind* of a type is
//! `Type` (concrete) or `Type → Type` (a type constructor). The
//! existing `kind_inference` module covers this fragment for HKT
//! support. **Polymorphic kinds** lift the kind language itself
//! into the polymorphic fragment: kind variables `κ`, kind
//! quantification `∀κ. K`, and kind unification across them.
//!
//! Practical use cases:
//!
//! * **Heterogeneous data types** — `data HList :: forall κ. [κ] → Type`.
//! * **Levitated functor combinators** — `Functor :: forall κ. (κ → κ) → Constraint`.
//! * **Type-class hierarchies parameterised by kind** — Haskell's
//!   `PolyKinds` extension.
//!
//! ## Algebra
//!
//! Kinds form a small algebra:
//!
//! ```text
//!     K ::= Type            (kind of value-level types)
//!         | K₁ → K₂         (kind of type constructors)
//!         | κ               (kind variable — substitutable)
//!         | Constraint      (kind of typeclass constraints)
//! ```
//!
//! Unification is structural: arrows match arrows, constants match
//! constants, and a kind variable `κ` unifies with any kind `K`
//! by substitution.
//!
//! ## API
//!
//! * [`Kind`] — the kind algebra.
//! * [`KindSubst`] — a substitution `κ ↦ K`.
//! * [`KindUnifier`] — accumulates substitutions, with `unify`
//!   merging two kinds and updating its substitution map.
//! * [`KindError`] — returned on unsolvable mismatches or occurs
//!   check failures.

use std::collections::HashMap;

use verum_common::Text;

/// A kind in the polymorphic-kind algebra.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Kind {
    /// `Type` — the kind of value-level types.
    Type,
    /// `Constraint` — the kind of typeclass constraints.
    Constraint,
    /// `K₁ → K₂` — a type constructor's kind.
    Arrow(Box<Kind>, Box<Kind>),
    /// `κ` — a kind variable, identified by name.
    Var(Text),
}

impl Kind {
    pub fn arrow(a: Kind, b: Kind) -> Self {
        Self::Arrow(Box::new(a), Box::new(b))
    }

    pub fn var(name: impl Into<Text>) -> Self {
        Self::Var(name.into())
    }

    /// Apply a substitution everywhere in this kind.
    pub fn apply(&self, subst: &KindSubst) -> Kind {
        match self {
            Kind::Var(name) => match subst.get(name) {
                Some(k) => k.apply(subst),
                None => self.clone(),
            },
            Kind::Arrow(a, b) => {
                Kind::Arrow(Box::new(a.apply(subst)), Box::new(b.apply(subst)))
            }
            Kind::Type | Kind::Constraint => self.clone(),
        }
    }

    /// Does the named kind variable occur anywhere in this kind?
    /// Used for the occurs check during unification.
    pub fn occurs(&self, var: &Text) -> bool {
        match self {
            Kind::Var(n) => n == var,
            Kind::Arrow(a, b) => a.occurs(var) || b.occurs(var),
            Kind::Type | Kind::Constraint => false,
        }
    }

    /// Free kind variables, in deterministic order of first
    /// appearance.
    pub fn free_vars(&self) -> Vec<Text> {
        let mut out = Vec::new();
        self.free_vars_into(&mut out);
        out
    }

    fn free_vars_into(&self, out: &mut Vec<Text>) {
        match self {
            Kind::Var(n) => {
                if !out.contains(n) {
                    out.push(n.clone());
                }
            }
            Kind::Arrow(a, b) => {
                a.free_vars_into(out);
                b.free_vars_into(out);
            }
            Kind::Type | Kind::Constraint => {}
        }
    }
}

impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Kind::Type => write!(f, "Type"),
            Kind::Constraint => write!(f, "Constraint"),
            Kind::Arrow(a, b) => match a.as_ref() {
                Kind::Arrow(_, _) => write!(f, "({}) → {}", a, b),
                _ => write!(f, "{} → {}", a, b),
            },
            Kind::Var(n) => write!(f, "{}", n.as_str()),
        }
    }
}

/// A kind substitution: kind variable name → kind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KindSubst {
    bindings: HashMap<Text, Kind>,
}

impl KindSubst {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn singleton(name: impl Into<Text>, k: Kind) -> Self {
        let mut s = Self::new();
        s.bindings.insert(name.into(), k);
        s
    }

    pub fn get(&self, name: &Text) -> Option<&Kind> {
        self.bindings.get(name)
    }

    pub fn insert(&mut self, name: Text, k: Kind) {
        self.bindings.insert(name, k);
    }

    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// Compose `other ∘ self` — apply `self` first, then `other`.
    /// Existing bindings are updated by `other`, and `other`'s
    /// bindings are also added.
    pub fn compose(&self, other: &KindSubst) -> KindSubst {
        let mut out = KindSubst::new();
        for (k, v) in &self.bindings {
            out.bindings.insert(k.clone(), v.apply(other));
        }
        for (k, v) in &other.bindings {
            out.bindings.entry(k.clone()).or_insert_with(|| v.clone());
        }
        out
    }
}

/// A kind unification error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KindError {
    /// Concrete kinds disagree (e.g., `Type` vs `Constraint`).
    Mismatch { left: Kind, right: Kind },
    /// Occurs check failed: would create an infinite kind.
    InfiniteKind { var: Text, kind: Kind },
    /// Arrow arity disagreement.
    ArityMismatch { left: Kind, right: Kind },
}

impl std::fmt::Display for KindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mismatch { left, right } => {
                write!(f, "kind mismatch: {} vs {}", left, right)
            }
            Self::InfiniteKind { var, kind } => write!(
                f,
                "occurs check failed: {} would equal {}",
                var.as_str(),
                kind
            ),
            Self::ArityMismatch { left, right } => {
                write!(f, "kind arity mismatch: {} vs {}", left, right)
            }
        }
    }
}

impl std::error::Error for KindError {}

/// Unify two kinds, accumulating bindings into a substitution.
pub fn unify(a: &Kind, b: &Kind) -> Result<KindSubst, KindError> {
    match (a, b) {
        // Same concrete kinds — empty substitution suffices.
        (Kind::Type, Kind::Type) => Ok(KindSubst::new()),
        (Kind::Constraint, Kind::Constraint) => Ok(KindSubst::new()),

        // Kind variable on either side.
        (Kind::Var(name), other) | (other, Kind::Var(name)) => {
            if let Kind::Var(n) = other {
                if n == name {
                    return Ok(KindSubst::new());
                }
            }
            if other.occurs(name) {
                return Err(KindError::InfiniteKind {
                    var: name.clone(),
                    kind: other.clone(),
                });
            }
            Ok(KindSubst::singleton(name.clone(), other.clone()))
        }

        // Arrows: unify components.
        (Kind::Arrow(a1, b1), Kind::Arrow(a2, b2)) => {
            let s1 = unify(a1, a2)?;
            let b1p = b1.apply(&s1);
            let b2p = b2.apply(&s1);
            let s2 = unify(&b1p, &b2p)?;
            Ok(s1.compose(&s2))
        }

        // Concrete arrow vs non-arrow.
        (l @ Kind::Arrow(_, _), r) | (l, r @ Kind::Arrow(_, _)) => {
            Err(KindError::ArityMismatch {
                left: l.clone(),
                right: r.clone(),
            })
        }

        // Anything else is a hard mismatch.
        _ => Err(KindError::Mismatch {
            left: a.clone(),
            right: b.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> Kind {
        Kind::Type
    }

    fn c() -> Kind {
        Kind::Constraint
    }

    fn k(name: &str) -> Kind {
        Kind::var(name)
    }

    #[test]
    fn type_unifies_with_type() {
        assert!(unify(&t(), &t()).unwrap().is_empty());
    }

    #[test]
    fn constraint_unifies_with_constraint() {
        assert!(unify(&c(), &c()).unwrap().is_empty());
    }

    #[test]
    fn type_does_not_unify_with_constraint() {
        assert!(matches!(
            unify(&t(), &c()),
            Err(KindError::Mismatch { .. })
        ));
    }

    #[test]
    fn var_unifies_with_concrete_kind() {
        let s = unify(&k("a"), &t()).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s.get(&Text::from("a")), Some(&Kind::Type));
    }

    #[test]
    fn var_unifies_with_self_via_no_substitution() {
        let s = unify(&k("a"), &k("a")).unwrap();
        assert!(s.is_empty());
    }

    #[test]
    fn occurs_check_rejects_self_arrow() {
        // a ~ a → Type would make a infinite.
        let r = unify(&k("a"), &Kind::arrow(k("a"), t()));
        assert!(matches!(r, Err(KindError::InfiniteKind { .. })));
    }

    #[test]
    fn arrow_unification_decomposes() {
        // (a → b) ~ (Type → Constraint)  ↦  a := Type, b := Constraint
        let lhs = Kind::arrow(k("a"), k("b"));
        let rhs = Kind::arrow(t(), c());
        let s = unify(&lhs, &rhs).unwrap();
        assert_eq!(s.get(&Text::from("a")), Some(&Kind::Type));
        assert_eq!(s.get(&Text::from("b")), Some(&Kind::Constraint));
    }

    #[test]
    fn arrow_vs_non_arrow_arity_error() {
        let r = unify(&Kind::arrow(t(), t()), &t());
        assert!(matches!(r, Err(KindError::ArityMismatch { .. })));
    }

    #[test]
    fn apply_substitutes_in_kind() {
        let kind = Kind::arrow(k("a"), Kind::arrow(k("b"), k("a")));
        let mut s = KindSubst::new();
        s.insert(Text::from("a"), Kind::Type);
        s.insert(Text::from("b"), Kind::Constraint);
        let result = kind.apply(&s);
        assert_eq!(
            result,
            Kind::arrow(t(), Kind::arrow(c(), t()))
        );
    }

    #[test]
    fn compose_substitutions_chains_apply() {
        // s1: a ↦ b
        // s2: b ↦ Type
        // composed: a ↦ Type, b ↦ Type
        let s1 = KindSubst::singleton("a", k("b"));
        let s2 = KindSubst::singleton("b", t());
        let composed = s1.compose(&s2);
        assert_eq!(composed.get(&Text::from("a")), Some(&t()));
        assert_eq!(composed.get(&Text::from("b")), Some(&t()));
    }

    #[test]
    fn occurs_predicate_finds_var_in_arrow() {
        let kind = Kind::arrow(t(), k("x"));
        assert!(kind.occurs(&Text::from("x")));
        assert!(!kind.occurs(&Text::from("y")));
    }

    #[test]
    fn free_vars_in_order_of_appearance() {
        let kind = Kind::arrow(k("a"), Kind::arrow(k("b"), k("a")));
        let fvs = kind.free_vars();
        assert_eq!(fvs.len(), 2);
        assert_eq!(fvs[0].as_str(), "a");
        assert_eq!(fvs[1].as_str(), "b");
    }

    #[test]
    fn free_vars_skips_concrete() {
        let kind = Kind::arrow(t(), c());
        assert!(kind.free_vars().is_empty());
    }

    #[test]
    fn display_uses_arrow_with_paren_for_left_arrow() {
        let kind = Kind::arrow(Kind::arrow(t(), t()), t());
        let s = format!("{}", kind);
        assert!(s.contains("(Type → Type) → Type") || s.contains("(Type"));
    }

    #[test]
    fn display_constants_render_correctly() {
        assert_eq!(format!("{}", t()), "Type");
        assert_eq!(format!("{}", c()), "Constraint");
        assert_eq!(format!("{}", k("a")), "a");
    }

    #[test]
    fn unify_double_var_substitution_chains() {
        // (a → b) ~ (b → Type)
        // s1: a ~ b ↦ a := b
        // applied: b → b ~ b → Type
        // s2: b := Type
        // composed: a ↦ Type, b ↦ Type
        let lhs = Kind::arrow(k("a"), k("b"));
        let rhs = Kind::arrow(k("b"), t());
        let s = unify(&lhs, &rhs).unwrap();
        assert_eq!(lhs.apply(&s), Kind::arrow(t(), t()));
        assert_eq!(rhs.apply(&s), Kind::arrow(t(), t()));
    }
}
