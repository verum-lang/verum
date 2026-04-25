// Shared OWL 2 graph + walker (VUVA §21).
//
// Consumed by:
//   - `commands::audit::audit_owl2_classify_with_format` (F5) — graph
//     audit producing the classification report.
//   - `commands::export` (B5) — OWL 2 Functional Syntax emitter that
//     reads the same graph + emits a Pellet/HermiT-compatible
//     `.ofn` file.
//
// Single source of truth for the `Owl2*Attr` → `Owl2Graph` projection.
// Both consumers run the same project walk and receive the same
// canonical graph; deterministic ordering (BTreeMap / BTreeSet) keeps
// CI-friendly diffs identical regardless of file-discovery order.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use verum_ast::attr::{
    Owl2Characteristic,
    Owl2CharacteristicAttr,
    Owl2ClassAttr,
    Owl2DisjointWithAttr,
    Owl2EquivalentClassAttr,
    Owl2HasKeyAttr,
    Owl2PropertyAttr,
    Owl2Semantics,
    Owl2SubClassOfAttr,
};
use verum_ast::decl::ItemKind;
use verum_ast::Item;
use verum_common::{Maybe, Text};

/// Kind of OWL 2 entity discovered at audit time. Class declarations
/// arrive through `Owl2ClassAttr`; properties through
/// `Owl2PropertyAttr` (with a domain/range pair) or
/// `Owl2CharacteristicAttr` (flag-only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Owl2EntityKind {
    Class,
    Property,
}

impl Owl2EntityKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Class    => "class",
            Self::Property => "property",
        }
    }
}

/// One declared OWL 2 entity with the metadata the walker collected.
#[derive(Debug, Clone)]
pub struct Owl2Entity {
    pub name: Text,
    pub kind: Owl2EntityKind,
    /// Open-world flag (only for classes). `None` ⇒ default ClosedWorld
    /// per VUVA §21.4.
    pub semantics: Option<Owl2Semantics>,
    /// Source file the entity was declared in.
    pub file: PathBuf,
    /// Property metadata — only populated when kind == Property.
    pub property_domain: Option<Text>,
    pub property_range:  Option<Text>,
    pub property_inverse_of: Option<Text>,
    pub property_characteristics: BTreeSet<Owl2Characteristic>,
    /// Has-key constraints (only for classes).
    pub keys: Vec<Vec<Text>>,
}

impl Owl2Entity {
    pub fn new_class(
        name: Text,
        semantics: Option<Owl2Semantics>,
        file: PathBuf,
    ) -> Self {
        Self {
            name, kind: Owl2EntityKind::Class, semantics, file,
            property_domain: None, property_range: None, property_inverse_of: None,
            property_characteristics: BTreeSet::new(),
            keys: Vec::new(),
        }
    }

    pub fn new_property(
        name: Text,
        file: PathBuf,
        domain: Option<Text>,
        range: Option<Text>,
        inverse_of: Option<Text>,
        characteristics: BTreeSet<Owl2Characteristic>,
    ) -> Self {
        Self {
            name, kind: Owl2EntityKind::Property, semantics: None, file,
            property_domain: domain,
            property_range:  range,
            property_inverse_of: inverse_of,
            property_characteristics: characteristics,
            keys: Vec::new(),
        }
    }
}

/// The OWL 2 classification graph: entities indexed by name, plus the
/// raw edge sets. Subclass / equivalence / disjointness are stored as
/// `BTreeSet<(Name, Name)>` for deterministic ordering and dedup.
/// Equivalence and disjointness pairs are stored *symmetrised* (both
/// (a,b) and (b,a) for every declared pair) so the closure walkers
/// don't have to handle orientation.
#[derive(Debug, Default)]
pub struct Owl2Graph {
    pub entities: BTreeMap<Text, Owl2Entity>,
    /// `subclass_edges[(child, parent)]` — direct edges only.
    pub subclass_edges: BTreeSet<(Text, Text)>,
    /// `equivalence_pairs[(class_a, class_b)]` — symmetric pairs.
    pub equivalence_pairs: BTreeSet<(Text, Text)>,
    /// `disjoint_pairs[(class_a, class_b)]` — symmetric pairs.
    pub disjoint_pairs: BTreeSet<(Text, Text)>,
}

impl Owl2Graph {
    /// Insert an entity, merging fields if a record under the same
    /// name already exists. Property characteristics merge from a
    /// flag-only `@owl2_characteristic` block into a richer
    /// `@owl2_property` block on the same fn; class keys append from a
    /// trailing `@owl2_has_key` onto the original `@owl2_class` record.
    pub fn add_entity(&mut self, e: Owl2Entity) {
        let key = e.name.clone();
        match self.entities.get_mut(&key) {
            Some(existing) if matches!(existing.kind, Owl2EntityKind::Property) => {
                for c in &e.property_characteristics {
                    existing.property_characteristics.insert(*c);
                }
                if existing.property_domain.is_none()    { existing.property_domain    = e.property_domain;    }
                if existing.property_range.is_none()     { existing.property_range     = e.property_range;     }
                if existing.property_inverse_of.is_none(){ existing.property_inverse_of= e.property_inverse_of;}
            }
            Some(existing) if matches!(existing.kind, Owl2EntityKind::Class) => {
                for k in &e.keys {
                    existing.keys.push(k.clone());
                }
                if existing.semantics.is_none() && e.semantics.is_some() {
                    existing.semantics = e.semantics;
                }
            }
            _ => {
                self.entities.insert(key, e);
            }
        }
    }

    /// Compute the *reflexive-transitive* subclass closure: each class
    /// maps to itself + every (transitive) ancestor. Iterative
    /// fixed-point — guaranteed to terminate because the lattice of
    /// possible ancestor sets is finite.
    pub fn subclass_closure(&self) -> BTreeMap<Text, BTreeSet<Text>> {
        let mut closure: BTreeMap<Text, BTreeSet<Text>> = BTreeMap::new();
        for (name, e) in &self.entities {
            if matches!(e.kind, Owl2EntityKind::Class) {
                let mut s = BTreeSet::new();
                s.insert(name.clone());
                closure.insert(name.clone(), s);
            }
        }
        loop {
            let mut changed = false;
            for (child, parent) in &self.subclass_edges {
                let parent_anc = closure.get(parent).cloned().unwrap_or_default();
                let entry = closure.entry(child.clone()).or_default();
                for a in parent_anc {
                    if entry.insert(a) {
                        changed = true;
                    }
                }
            }
            if !changed { break; }
        }
        closure
    }

    /// Detect subclass cycles by walking the closure. Returns the
    /// names of cyclic classes (both halves of every cycle).
    pub fn detect_cycles(
        &self,
        closure: &BTreeMap<Text, BTreeSet<Text>>,
    ) -> BTreeSet<Text> {
        let mut cyclic: BTreeSet<Text> = BTreeSet::new();
        for (child, parent) in &self.subclass_edges {
            if child == parent {
                cyclic.insert(child.clone());
                continue;
            }
            if let Some(p_anc) = closure.get(parent) {
                if p_anc.contains(child) {
                    cyclic.insert(child.clone());
                    cyclic.insert(parent.clone());
                }
            }
        }
        cyclic
    }

    /// Compute the equivalence partition by union-find over
    /// `equivalence_pairs`. Each entry in the result is a set of
    /// classes pairwise equivalent (size ≥ 2; singletons omitted).
    pub fn equivalence_partition(&self) -> Vec<BTreeSet<Text>> {
        let mut parent: BTreeMap<Text, Text> = BTreeMap::new();
        for (a, b) in &self.equivalence_pairs {
            parent.entry(a.clone()).or_insert_with(|| a.clone());
            parent.entry(b.clone()).or_insert_with(|| b.clone());
        }
        fn find(parent: &mut BTreeMap<Text, Text>, x: &Text) -> Text {
            let p = parent.get(x).cloned().unwrap_or_else(|| x.clone());
            if &p == x { return p; }
            let root = find(parent, &p);
            parent.insert(x.clone(), root.clone());
            root
        }
        for (a, b) in &self.equivalence_pairs {
            let ra = find(&mut parent, a);
            let rb = find(&mut parent, b);
            if ra != rb {
                parent.insert(ra, rb);
            }
        }
        let mut groups: BTreeMap<Text, BTreeSet<Text>> = BTreeMap::new();
        let keys: Vec<Text> = parent.keys().cloned().collect();
        for k in keys {
            let r = find(&mut parent, &k);
            groups.entry(r).or_default().insert(k);
        }
        groups.into_values().filter(|g| g.len() > 1).collect()
    }

    /// Detect disjoint/subclass conflicts: a class C is disjoint from
    /// D, but C is also a subclass of D (directly or transitively).
    /// This is a hard inconsistency (DL-unsatisfiable ontology).
    pub fn detect_disjoint_violations(
        &self,
        closure: &BTreeMap<Text, BTreeSet<Text>>,
    ) -> BTreeSet<(Text, Text)> {
        let mut violations: BTreeSet<(Text, Text)> = BTreeSet::new();
        for (a, b) in &self.disjoint_pairs {
            if a == b {
                violations.insert((a.clone(), b.clone()));
                continue;
            }
            if let Some(a_anc) = closure.get(a) {
                if a_anc.contains(b) {
                    violations.insert((a.clone(), b.clone()));
                }
            }
        }
        violations
    }

    /// True if the graph contains any OWL 2 entity (class or property).
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }
}

/// Walk one parsed Item, classify all `@owl2_*` markers it carries,
/// and feed entities + edges into the graph. Walks both
/// `item.attributes` (outer) and the inner decl.attributes (per
/// kind) to mirror the framework / hygiene audit collection paths.
pub fn collect_owl2_attrs(
    item: &Item,
    rel_path: &Path,
    graph: &mut Owl2Graph,
) {
    let (item_name, decl_attrs): (Text, &verum_common::List<verum_ast::attr::Attribute>) = match &item.kind {
        ItemKind::Type(decl)     => (decl.name.name.clone(), &decl.attributes),
        ItemKind::Function(decl) => (decl.name.name.clone(), &decl.attributes),
        ItemKind::Theorem(decl) | ItemKind::Lemma(decl) | ItemKind::Corollary(decl) => {
            (decl.name.name.clone(), &decl.attributes)
        }
        ItemKind::Axiom(decl) => (decl.name.name.clone(), &decl.attributes),
        _ => return,
    };

    let attr_lists = [&item.attributes, decl_attrs];

    let mut is_class = false;
    let mut class_semantics: Option<Owl2Semantics> = None;
    let mut subclass_parents: Vec<Text> = Vec::new();
    let mut equivalent_classes: Vec<Text> = Vec::new();
    let mut disjoint_classes:  Vec<Text> = Vec::new();
    let mut keys: Vec<Vec<Text>> = Vec::new();
    let mut is_property = false;
    let mut prop_domain: Option<Text> = None;
    let mut prop_range:  Option<Text> = None;
    let mut prop_inverse: Option<Text> = None;
    let mut prop_chars: BTreeSet<Owl2Characteristic> = BTreeSet::new();

    for list in &attr_lists {
        for attr in list.iter() {
            if let Maybe::Some(c) = Owl2ClassAttr::from_attribute(attr) {
                is_class = true;
                if let Maybe::Some(sem) = c.semantics {
                    class_semantics = Some(sem);
                }
            }
            if let Maybe::Some(s) = Owl2SubClassOfAttr::from_attribute(attr) {
                is_class = true;
                subclass_parents.push(s.parent);
            }
            if let Maybe::Some(e) = Owl2EquivalentClassAttr::from_attribute(attr) {
                is_class = true;
                equivalent_classes.push(e.equivalent_to);
            }
            if let Maybe::Some(d) = Owl2DisjointWithAttr::from_attribute(attr) {
                is_class = true;
                for n in d.disjoint_classes {
                    disjoint_classes.push(n);
                }
            }
            if let Maybe::Some(k) = Owl2HasKeyAttr::from_attribute(attr) {
                is_class = true;
                keys.push(k.key_properties);
            }
            if let Maybe::Some(p) = Owl2PropertyAttr::from_attribute(attr) {
                is_property = true;
                if let Maybe::Some(d) = p.domain     { prop_domain  = Some(d); }
                if let Maybe::Some(r) = p.range      { prop_range   = Some(r); }
                if let Maybe::Some(i) = p.inverse_of { prop_inverse = Some(i); }
                for c in p.characteristics { prop_chars.insert(c); }
            }
            if let Maybe::Some(c) = Owl2CharacteristicAttr::from_attribute(attr) {
                is_property = true;
                prop_chars.insert(c.characteristic);
            }
        }
    }

    if is_class {
        let mut entity = Owl2Entity::new_class(
            item_name.clone(),
            class_semantics,
            rel_path.to_path_buf(),
        );
        entity.keys = keys;
        graph.add_entity(entity);
        for p in subclass_parents {
            graph.subclass_edges.insert((item_name.clone(), p));
        }
        for e in equivalent_classes {
            graph.equivalence_pairs.insert((item_name.clone(), e.clone()));
            graph.equivalence_pairs.insert((e, item_name.clone()));
        }
        for d in disjoint_classes {
            graph.disjoint_pairs.insert((item_name.clone(), d.clone()));
            graph.disjoint_pairs.insert((d, item_name.clone()));
        }
    }
    if is_property {
        let entity = Owl2Entity::new_property(
            item_name, rel_path.to_path_buf(),
            prop_domain, prop_range, prop_inverse, prop_chars,
        );
        graph.add_entity(entity);
    }
}
