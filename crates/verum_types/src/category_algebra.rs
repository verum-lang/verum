//! Category-Theoretic Morphism Algebra.
//!
//! A *category* consists of objects and morphisms (arrows between
//! objects), with composition that is associative and respects
//! identity. This module provides a finite, explicitly tabulated
//! category core: a small object set, a list of morphisms typed
//! by source/target, a composition table, and verification of
//! the categorical laws.
//!
//! The `core/math/category.vr` stdlib module
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
//!  id_B ∘ f = f (left identity)
//!  f ∘ id_A = f (right identity)
//!  h ∘ (g ∘ f) = (h ∘ g) ∘ f (associativity)
//! ```
//!
//! ## API
//!
//! * [`Object`] — categorical object identifier.
//! * [`Morphism`] — typed arrow `source → target`.
//! * [`Category`] — finite presentation (objects, morphisms,
//!  identities, composition table).
//! * [`Category::compose`] — look up a declared composition.
//! * [`Category::check_laws`] — check totality, endpoints and all category laws.

use verum_common::{List, OrderedMap, OrderedSet, Text};

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
        write!(
            f,
            "{} : {} → {}",
            self.name.as_str(),
            self.source,
            self.target
        )
    }
}

/// A finite category presentation.
#[derive(Debug, Clone, Default)]
pub struct Category {
    objects: OrderedSet<Object>,
    morphisms: OrderedMap<Text, Morphism>,
    /// Identity morphism for each object, by object name.
    identities: OrderedMap<Object, Text>,
    /// Composition table: g ↦ (f ↦ g ∘ f), keyed by morphism name.
    /// Nested keys permit borrowed lookups without allocating key pairs.
    composition: OrderedMap<Text, OrderedMap<Text, Text>>,
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
    pub fn set_identity(&mut self, obj: Object, m: impl Into<Text>) -> Result<(), CategoryError> {
        let name = m.into();
        self.check_identity_shape(&obj, &name)?;
        self.identities.insert(obj, name);
        Ok(())
    }

    fn check_identity_shape(&self, obj: &Object, name: &Text) -> Result<(), CategoryError> {
        let morph = self
            .morphisms
            .get(name)
            .ok_or_else(|| CategoryError::UnknownMorphism { name: name.clone() })?;
        if &morph.source != obj || &morph.target != obj {
            return Err(CategoryError::IdentityShape { name: name.clone() });
        }
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
        self.check_composition_shape(&g_name, &f_name, &res_name)?;
        self.composition
            .entry(g_name)
            .or_default()
            .insert(f_name, res_name);
        Ok(())
    }

    fn check_composition_shape(
        &self,
        g_name: &Text,
        f_name: &Text,
        res_name: &Text,
    ) -> Result<(), CategoryError> {
        let gm = self
            .morphisms
            .get(g_name)
            .ok_or_else(|| CategoryError::UnknownMorphism {
                name: g_name.clone(),
            })?;
        let fm = self
            .morphisms
            .get(f_name)
            .ok_or_else(|| CategoryError::UnknownMorphism {
                name: f_name.clone(),
            })?;
        if fm.target != gm.source {
            return Err(CategoryError::CompositionTypeMismatch {
                g: g_name.clone(),
                f: f_name.clone(),
            });
        }
        let rm = self
            .morphisms
            .get(res_name)
            .ok_or_else(|| CategoryError::UnknownMorphism {
                name: res_name.clone(),
            })?;
        if rm.source != fm.source || rm.target != gm.target {
            return Err(CategoryError::CompositionResultShape {
                result: res_name.clone(),
            });
        }
        Ok(())
    }

    /// Look up `g ∘ f`.
    pub fn compose(&self, g: &Text, f: &Text) -> Option<&Text> {
        self.composition.get(g)?.get(f)
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

    /// Check the current finite presentation, including totality and endpoints.
    /// Setter validation is insufficient because an arrow can later be replaced.
    /// Success describes this immutable snapshot; it is not a proof of a category
    /// with unenumerated objects or morphisms.
    pub fn check_laws(&self) -> Result<(), CategoryError> {
        for object in self.objects.iter() {
            let identity =
                self.identities
                    .get(object)
                    .ok_or_else(|| CategoryError::MissingIdentity {
                        object: object.clone(),
                    })?;
            self.check_identity_shape(object, identity)?;
        }
        for (g, row) in self.composition.iter() {
            for (f, result) in row.iter() {
                self.check_composition_shape(g, f, result)?;
            }
        }

        // Index by endpoints: only composable pairs and triples enter the laws.
        // Ordered iteration gives reproducible diagnostics without sorting or
        // cloning names in the cubic (one-object category) case.
        let mut outgoing: OrderedMap<&Object, List<&Morphism>> = OrderedMap::new();
        for arrow in self.morphisms.values() {
            outgoing.entry(&arrow.source).or_default().push(arrow);
        }
        for f in self.morphisms.values() {
            if let Some(successors) = outgoing.get(&&f.target) {
                for g in successors {
                    self.require_composition(&g.name, &f.name)?;
                }
            }
        }
        for f in self.morphisms.values() {
            // The first pass established an identity for every endpoint.
            let id_target = self.identities.get(&f.target).expect("checked identity");
            let id_source = self.identities.get(&f.source).expect("checked identity");
            if self.require_composition(id_target, &f.name)? != &f.name {
                return Err(CategoryError::LeftIdentityFailure {
                    morphism: f.name.clone(),
                });
            }
            if self.require_composition(&f.name, id_source)? != &f.name {
                return Err(CategoryError::RightIdentityFailure {
                    morphism: f.name.clone(),
                });
            }
            if let Some(successors) = outgoing.get(&&f.target) {
                for g in successors {
                    let gf = self.require_composition(&g.name, &f.name)?;
                    if let Some(thirds) = outgoing.get(&&g.target) {
                        for h in thirds {
                            let hg = self.require_composition(&h.name, &g.name)?;
                            let lhs = self.require_composition(&h.name, gf)?;
                            let rhs = self.require_composition(hg, &f.name)?;
                            if lhs != rhs {
                                return Err(CategoryError::AssociativityFailure {
                                    h: h.name.clone(),
                                    g: g.name.clone(),
                                    f: f.name.clone(),
                                });
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn require_composition(&self, g: &Text, f: &Text) -> Result<&Text, CategoryError> {
        self.compose(g, f)
            .ok_or_else(|| CategoryError::MissingComposition {
                g: g.clone(),
                f: f.clone(),
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CategoryError {
    MissingIdentity { object: Object },
    MissingComposition { g: Text, f: Text },
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
            Self::MissingIdentity { object } => write!(f, "object `{object}` has no identity"),
            Self::MissingComposition { g, f: ff } => {
                write!(f, "composition `{g} ∘ {ff}` is missing")
            }
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
        assert!(matches!(
            r,
            Err(CategoryError::CompositionTypeMismatch { .. })
        ));
    }

    #[test]
    fn set_composition_rejects_wrong_result_shape() {
        let mut c = Category::new();
        c.add_morphism(Morphism::new("f", obj("A"), obj("B")));
        c.add_morphism(Morphism::new("g", obj("B"), obj("C")));
        c.add_morphism(Morphism::new("h", obj("X"), obj("Y"))); // wrong shape
        let r = c.set_composition("g", "f", "h");
        assert!(matches!(
            r,
            Err(CategoryError::CompositionResultShape { .. })
        ));
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
        // A total one-object operation with a deliberately wrong left unit.
        let mut c = Category::new();
        c.add_morphism(Morphism::new("f", obj("A"), obj("A")));
        c.add_morphism(Morphism::new("id", obj("A"), obj("A")));
        c.set_identity(obj("A"), "id").unwrap();
        c.set_composition("id", "id", "id").unwrap();
        c.set_composition("id", "f", "id").unwrap();
        c.set_composition("f", "id", "f").unwrap();
        c.set_composition("f", "f", "f").unwrap();
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

        let arrows: List<_> = c.morphisms.values().cloned().collect();
        for object in ["A", "B", "C", "D"] {
            let id = Text::from(format!("id_{object}"));
            c.add_morphism(Morphism::new(id.clone(), obj(object), obj(object)));
            c.set_identity(obj(object), id.clone()).unwrap();
            c.set_composition(id.clone(), id.clone(), id).unwrap();
        }
        for arrow in arrows {
            c.set_composition(
                format!("id_{}", arrow.target),
                arrow.name.clone(),
                arrow.name.clone(),
            )
            .unwrap();
            c.set_composition(
                arrow.name.clone(),
                format!("id_{}", arrow.source),
                arrow.name,
            )
            .unwrap();
        }
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

        let arrows: List<_> = c.morphisms.values().cloned().collect();
        for object in ["A", "B", "C", "D"] {
            let id = Text::from(format!("id_{object}"));
            c.add_morphism(Morphism::new(id.clone(), obj(object), obj(object)));
            c.set_identity(obj(object), id.clone()).unwrap();
            c.set_composition(id.clone(), id.clone(), id).unwrap();
        }
        for arrow in arrows {
            c.set_composition(
                format!("id_{}", arrow.target),
                arrow.name.clone(),
                arrow.name.clone(),
            )
            .unwrap();
            c.set_composition(
                arrow.name.clone(),
                format!("id_{}", arrow.source),
                arrow.name,
            )
            .unwrap();
        }
        c.set_composition("g", "f", "gf").unwrap();
        c.set_composition("h", "g", "hg").unwrap();
        c.set_composition("h", "gf", "hgf").unwrap();
        c.set_composition("hg", "f", "hgf").unwrap();
        assert!(c.check_laws().is_ok());
    }
}
