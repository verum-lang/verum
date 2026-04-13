//! Category-Theoretic Morphism Algebra.
//!
//! A *category* consists of objects and morphisms (arrows between
//! objects), with composition that is associative and respects
//! identity. This module provides a finite, finitely-presented
//! category core: a small object set, a list of morphisms typed
//! by source/target, a composition table, and verification of
//! the categorical laws.
//!
//! The existing `core/math/category.vr` stdlib module (738 LoC)
//! describes categories at the protocol level. This module
//! complements it with a *concrete data structure* for working
//! with finite presentations programmatically — useful for
//! categorical SMT encodings, diagram chasing, and presheaf
//! algorithmics.
//!
//! ## Laws
//!
//! For all morphisms `f : A → B`, `g : B → C`, `h : C → D`:
//!
//! ```text
//!     id_B ∘ f = f         (left identity)
//!     f ∘ id_A = f         (right identity)
//!     h ∘ (g ∘ f) = (h ∘ g) ∘ f      (associativity)
//! ```
//!
//! ## API
//!
//! * [`Object`] — categorical object identifier.
//! * [`Morphism`] — typed arrow `source → target`.
//! * [`Category`] — finite presentation (objects, morphisms,
//!   identities, composition table).
//! * [`compose`] — categorical composition, with type-check.
//! * [`check_laws`] — verifies left/right identity + associativity.

use std::collections::{BTreeMap, BTreeSet};

use verum_common::Text;

/// A categorical object.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Object {
    pub name: Text,
}

impl Object {
    pub fn new(name: impl Into<Text>) -> Self {
        Self { name: name.into() }
    }
}

impl std::fmt::Display for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name.as_str())
    }
}

/// A morphism with explicit source and target.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Morphism {
    pub name: Text,
    pub source: Object,
    pub target: Object,
}

impl Morphism {
    pub fn new(name: impl Into<Text>, source: Object, target: Object) -> Self {
        Self {
            name: name.into(),
            source,
            target,
        }
    }
}

impl std::fmt::Display for Morphism {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} : {} → {}", self.name.as_str(), self.source, self.target)
    }
}

/// A finite category presentation.
#[derive(Debug, Clone, Default)]
pub struct Category {
    objects: BTreeSet<Object>,
    morphisms: BTreeMap<Text, Morphism>,
    /// Identity morphism for each object, by object name.
    identities: BTreeMap<Object, Text>,
    /// Composition table: (g, f) ↦ g ∘ f, both keyed by morphism name.
    composition: BTreeMap<(Text, Text), Text>,
}

impl Category {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_object(&mut self, obj: Object) {
        self.objects.insert(obj);
    }

    pub fn add_morphism(&mut self, m: Morphism) {
        self.objects.insert(m.source.clone());
        self.objects.insert(m.target.clone());
        self.morphisms.insert(m.name.clone(), m);
    }

    /// Declare that `m` is the identity morphism on `obj`.
    /// `m` must already be registered with `m.source = m.target = obj`.
    pub fn set_identity(
        &mut self,
        obj: Object,
        m: impl Into<Text>,
    ) -> Result<(), CategoryError> {
        let name = m.into();
        let morph = self.morphisms.get(&name).ok_or_else(|| CategoryError::UnknownMorphism {
            name: name.clone(),
        })?;
        if morph.source != obj || morph.target != obj {
            return Err(CategoryError::IdentityShape {
                name: name.clone(),
            });
        }
        self.identities.insert(obj, name);
        Ok(())
    }

    /// Declare `g ∘ f = result`. Both `f` and `g` must be
    /// registered, with `f.target == g.source` and the result's
    /// source/target must match (`source = f.source`, `target = g.target`).
    pub fn set_composition(
        &mut self,
        g: impl Into<Text>,
        f: impl Into<Text>,
        result: impl Into<Text>,
    ) -> Result<(), CategoryError> {
        let g_name = g.into();
        let f_name = f.into();
        let res_name = result.into();
        let gm = self
            .morphisms
            .get(&g_name)
            .ok_or_else(|| CategoryError::UnknownMorphism { name: g_name.clone() })?;
        let fm = self
            .morphisms
            .get(&f_name)
            .ok_or_else(|| CategoryError::UnknownMorphism { name: f_name.clone() })?;
        if fm.target != gm.source {
            return Err(CategoryError::CompositionTypeMismatch {
                g: g_name,
                f: f_name,
            });
        }
        let rm = self.morphisms.get(&res_name).ok_or_else(|| {
            CategoryError::UnknownMorphism { name: res_name.clone() }
        })?;
        if rm.source != fm.source || rm.target != gm.target {
            return Err(CategoryError::CompositionResultShape {
                result: res_name,
            });
        }
        self.composition.insert((g_name, f_name), res_name);
        Ok(())
    }

    /// Look up `g ∘ f`.
    pub fn compose(&self, g: &Text, f: &Text) -> Option<&Text> {
        self.composition.get(&(g.clone(), f.clone()))
    }

    /// Identity morphism for an object.
    pub fn identity_of(&self, obj: &Object) -> Option<&Text> {
        self.identities.get(obj)
    }

    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    pub fn morphism_count(&self) -> usize {
        self.morphisms.len()
    }

    /// Verify the categorical laws on this finite presentation.
    /// Returns the first violation found (alphabetical ordering)
    /// or `Ok(())` if every law holds.
    pub fn check_laws(&self) -> Result<(), CategoryError> {
        // Left identity: id_B ∘ f = f for every f : A → B.
        for f in self.morphisms.values() {
            let id_target = self.identities.get(&f.target).cloned();
            if let Some(id_t) = id_target {
                let composed = self.compose(&id_t, &f.name);
                if composed != Some(&f.name) {
                    return Err(CategoryError::LeftIdentityFailure {
                        morphism: f.name.clone(),
                    });
                }
            }
        }
        // Right identity: f ∘ id_A = f for every f : A → B.
        for f in self.morphisms.values() {
            let id_source = self.identities.get(&f.source).cloned();
            if let Some(id_s) = id_source {
                let composed = self.compose(&f.name, &id_s);
                if composed != Some(&f.name) {
                    return Err(CategoryError::RightIdentityFailure {
                        morphism: f.name.clone(),
                    });
                }
            }
        }
        // Associativity: h ∘ (g ∘ f) = (h ∘ g) ∘ f for every
        // composable triple. We iterate alphabetically for
        // determinism.
        let mut names: Vec<&Text> = self.morphisms.keys().collect();
        names.sort();
        for h in &names {
            for g in &names {
                for f in &names {
                    let gf = self.compose(g, f).cloned();
                    let hg = self.compose(h, g).cloned();
                    if let (Some(gf), Some(hg)) = (gf, hg) {
                        let lhs = self.compose(h, &gf).cloned();
                        let rhs = self.compose(&hg, f).cloned();
                        if lhs.is_some() && rhs.is_some() && lhs != rhs {
                            return Err(CategoryError::AssociativityFailure {
                                h: (**h).clone(),
                                g: (**g).clone(),
                                f: (**f).clone(),
                            });
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CategoryError {
    UnknownMorphism { name: Text },
    IdentityShape { name: Text },
    CompositionTypeMismatch { g: Text, f: Text },
    CompositionResultShape { result: Text },
    LeftIdentityFailure { morphism: Text },
    RightIdentityFailure { morphism: Text },
    AssociativityFailure { h: Text, g: Text, f: Text },
}

impl std::fmt::Display for CategoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownMorphism { name } => write!(f, "unknown morphism `{}`", name.as_str()),
            Self::IdentityShape { name } => write!(
                f,
                "morphism `{}` cannot be an identity (source ≠ target)",
                name.as_str()
            ),
            Self::CompositionTypeMismatch { g, f: ff } => write!(
                f,
                "cannot compose `{} ∘ {}`: target of {} ≠ source of {}",
                g.as_str(),
                ff.as_str(),
                ff.as_str(),
                g.as_str()
            ),
            Self::CompositionResultShape { result } => write!(
                f,
                "composition result `{}` has wrong source/target",
                result.as_str()
            ),
            Self::LeftIdentityFailure { morphism } => write!(
                f,
                "left-identity law fails for morphism `{}`",
                morphism.as_str()
            ),
            Self::RightIdentityFailure { morphism } => write!(
                f,
                "right-identity law fails for morphism `{}`",
                morphism.as_str()
            ),
            Self::AssociativityFailure { h, g, f: ff } => write!(
                f,
                "associativity fails for {} ∘ ({} ∘ {})",
                h.as_str(),
                g.as_str(),
                ff.as_str()
            ),
        }
    }
}

impl std::error::Error for CategoryError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(s: &str) -> Object {
        Object::new(s)
    }

    #[test]
    fn empty_category_has_no_objects() {
        let c = Category::new();
        assert_eq!(c.object_count(), 0);
        assert_eq!(c.morphism_count(), 0);
        assert!(c.check_laws().is_ok()); // vacuously true
    }

    #[test]
    fn add_morphism_registers_endpoints() {
        let mut c = Category::new();
        c.add_morphism(Morphism::new("f", obj("A"), obj("B")));
        assert_eq!(c.object_count(), 2);
        assert_eq!(c.morphism_count(), 1);
    }

    #[test]
    fn set_identity_requires_self_loop() {
        let mut c = Category::new();
        c.add_morphism(Morphism::new("f", obj("A"), obj("B")));
        let r = c.set_identity(obj("A"), "f");
        assert!(matches!(r, Err(CategoryError::IdentityShape { .. })));
    }

    #[test]
    fn set_identity_succeeds_for_endo() {
        let mut c = Category::new();
        c.add_morphism(Morphism::new("id_A", obj("A"), obj("A")));
        assert!(c.set_identity(obj("A"), "id_A").is_ok());
        assert_eq!(c.identity_of(&obj("A")).map(|t| t.as_str()), Some("id_A"));
    }

    #[test]
    fn set_composition_rejects_type_mismatch() {
        let mut c = Category::new();
        c.add_morphism(Morphism::new("f", obj("A"), obj("B")));
        c.add_morphism(Morphism::new("g", obj("C"), obj("D")));
        // f.target = B but g.source = C — mismatch.
        let r = c.set_composition("g", "f", "f");
        assert!(matches!(r, Err(CategoryError::CompositionTypeMismatch { .. })));
    }

    #[test]
    fn set_composition_rejects_wrong_result_shape() {
        let mut c = Category::new();
        c.add_morphism(Morphism::new("f", obj("A"), obj("B")));
        c.add_morphism(Morphism::new("g", obj("B"), obj("C")));
        c.add_morphism(Morphism::new("h", obj("X"), obj("Y"))); // wrong shape
        let r = c.set_composition("g", "f", "h");
        assert!(matches!(r, Err(CategoryError::CompositionResultShape { .. })));
    }

    #[test]
    fn compose_lookup_returns_result() {
        let mut c = Category::new();
        c.add_morphism(Morphism::new("f", obj("A"), obj("B")));
        c.add_morphism(Morphism::new("g", obj("B"), obj("C")));
        c.add_morphism(Morphism::new("gf", obj("A"), obj("C")));
        c.set_composition("g", "f", "gf").unwrap();
        let r = c.compose(&Text::from("g"), &Text::from("f"));
        assert_eq!(r.map(|t| t.as_str()), Some("gf"));
    }

    #[test]
    fn check_laws_passes_for_identity_only_category() {
        // Single object A with id_A, and id_A ∘ id_A = id_A.
        let mut c = Category::new();
        c.add_morphism(Morphism::new("id_A", obj("A"), obj("A")));
        c.set_identity(obj("A"), "id_A").unwrap();
        c.set_composition("id_A", "id_A", "id_A").unwrap();
        assert!(c.check_laws().is_ok());
    }

    #[test]
    fn check_laws_detects_left_identity_failure() {
        // f : A → B, id_B exists, but id_B ∘ f is *not* registered
        // as f. This is a missing-composition case rather than a
        // wrong one — left-identity should still flag.
        let mut c = Category::new();
        c.add_morphism(Morphism::new("f", obj("A"), obj("B")));
        c.add_morphism(Morphism::new("id_B", obj("B"), obj("B")));
        c.set_identity(obj("B"), "id_B").unwrap();
        // No id_B ∘ f registered → left-identity fails for f.
        let r = c.check_laws();
        assert!(matches!(r, Err(CategoryError::LeftIdentityFailure { .. })));
    }

    #[test]
    fn check_laws_passes_when_identity_compositions_registered() {
        let mut c = Category::new();
        c.add_morphism(Morphism::new("f", obj("A"), obj("B")));
        c.add_morphism(Morphism::new("id_A", obj("A"), obj("A")));
        c.add_morphism(Morphism::new("id_B", obj("B"), obj("B")));
        c.set_identity(obj("A"), "id_A").unwrap();
        c.set_identity(obj("B"), "id_B").unwrap();
        c.set_composition("id_B", "f", "f").unwrap();
        c.set_composition("f", "id_A", "f").unwrap();
        c.set_composition("id_A", "id_A", "id_A").unwrap();
        c.set_composition("id_B", "id_B", "id_B").unwrap();
        assert!(c.check_laws().is_ok());
    }

    #[test]
    fn morphism_display_shows_typed_arrow() {
        let m = Morphism::new("f", obj("A"), obj("B"));
        assert_eq!(format!("{}", m), "f : A → B");
    }

    #[test]
    fn unknown_morphism_in_set_identity() {
        let mut c = Category::new();
        let r = c.set_identity(obj("A"), "nope");
        assert!(matches!(r, Err(CategoryError::UnknownMorphism { .. })));
    }

    #[test]
    fn unknown_morphism_in_set_composition() {
        let mut c = Category::new();
        c.add_morphism(Morphism::new("f", obj("A"), obj("B")));
        let r = c.set_composition("missing", "f", "f");
        assert!(matches!(r, Err(CategoryError::UnknownMorphism { .. })));
    }

    #[test]
    fn associativity_failure_detected() {
        // f : A → B, g : B → C, h : C → D, but we register
        // h ∘ (g ∘ f) ≠ (h ∘ g) ∘ f.
        let mut c = Category::new();
        c.add_morphism(Morphism::new("f", obj("A"), obj("B")));
        c.add_morphism(Morphism::new("g", obj("B"), obj("C")));
        c.add_morphism(Morphism::new("h", obj("C"), obj("D")));
        c.add_morphism(Morphism::new("gf", obj("A"), obj("C")));
        c.add_morphism(Morphism::new("hg", obj("B"), obj("D")));
        c.add_morphism(Morphism::new("path1", obj("A"), obj("D")));
        c.add_morphism(Morphism::new("path2", obj("A"), obj("D")));

        c.set_composition("g", "f", "gf").unwrap();
        c.set_composition("h", "g", "hg").unwrap();
        c.set_composition("h", "gf", "path1").unwrap();
        c.set_composition("hg", "f", "path2").unwrap();
        // path1 ≠ path2 ⇒ associativity fails
        let r = c.check_laws();
        assert!(matches!(r, Err(CategoryError::AssociativityFailure { .. })));
    }

    #[test]
    fn associativity_passes_when_paths_agree() {
        let mut c = Category::new();
        c.add_morphism(Morphism::new("f", obj("A"), obj("B")));
        c.add_morphism(Morphism::new("g", obj("B"), obj("C")));
        c.add_morphism(Morphism::new("h", obj("C"), obj("D")));
        c.add_morphism(Morphism::new("gf", obj("A"), obj("C")));
        c.add_morphism(Morphism::new("hg", obj("B"), obj("D")));
        c.add_morphism(Morphism::new("hgf", obj("A"), obj("D")));

        c.set_composition("g", "f", "gf").unwrap();
        c.set_composition("h", "g", "hg").unwrap();
        c.set_composition("h", "gf", "hgf").unwrap();
        c.set_composition("hg", "f", "hgf").unwrap();
        assert!(c.check_laws().is_ok());
    }
}
